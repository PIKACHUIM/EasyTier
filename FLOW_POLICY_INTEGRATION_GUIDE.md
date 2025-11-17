# 流量策略执行逻辑集成指南

本文档说明如何将流量策略管理器集成到EasyTier的数据转发路径中。

## 集成概述

流量策略需要在以下几个关键位置进行检查和执行：

1. **数据发送前** - 检查带宽限制
2. **中转决策时** - 检查是否禁用中转
3. **公共转发时** - 检查是否禁用公共转发

## 集成方案

### 方案1：通过GlobalCtx传递（推荐）

在GlobalCtx中添加flow_policy_manager的弱引用，这样可以在整个系统中访问策略管理器。

#### 步骤1：修改GlobalCtx结构

文件：`easytier/src/common/global_ctx.rs`

```rust
use std::sync::Weak;
use crate::common::flow_policy_manager::FlowPolicyManager;

pub struct GlobalCtx {
    // ... 现有字段 ...
    
    flow_policy_manager: Mutex<Option<Weak<FlowPolicyManager>>>,
    report_manager: Mutex<Option<Weak<ReportManager>>>,
}

impl GlobalCtx {
    pub fn new(config_fs: impl ConfigLoader + 'static) -> Self {
        // ... 现有初始化代码 ...
        
        GlobalCtx {
            // ... 现有字段初始化 ...
            flow_policy_manager: Mutex::new(None),
            report_manager: Mutex::new(None),
        }
    }
    
    pub fn set_flow_policy_manager(&self, manager: Option<Weak<FlowPolicyManager>>) {
        *self.flow_policy_manager.lock().unwrap() = manager;
    }
    
    pub fn get_flow_policy_manager(&self) -> Option<Arc<FlowPolicyManager>> {
        self.flow_policy_manager
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }
    
    pub fn set_report_manager(&self, manager: Option<Weak<ReportManager>>) {
        *self.report_manager.lock().unwrap() = manager;
    }
    
    pub fn get_report_manager(&self) -> Option<Arc<ReportManager>> {
        self.report_manager
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }
}
```

#### 步骤2：在Instance初始化时设置管理器

文件：`easytier/src/launcher.rs`

在`easytier_routine`方法中，Instance运行后设置管理器：

```rust
async fn easytier_routine(
    cfg: TomlConfigLoader,
    stop_signal: Arc<tokio::sync::Notify>,
    api_service: ArcMutApiService,
    data: Arc<EasyTierData>,
) -> Result<(), anyhow::Error> {
    let mut instance = Instance::new(cfg);
    let mut tasks = JoinSet::new();

    // ... 现有代码 ...

    instance.run().await?;

    // 初始化流量策略管理器和上报管理器
    // 注意：这些配置需要通过API动态设置
    let global_ctx = instance.get_global_ctx();
    
    // 如果有flow_policy_manager，设置到GlobalCtx
    if let Some(manager) = instance.get_flow_policy_manager() {
        global_ctx.set_flow_policy_manager(Some(Arc::downgrade(&manager)));
    }
    
    // 如果有report_manager，设置到GlobalCtx
    if let Some(manager) = instance.get_report_manager() {
        global_ctx.set_report_manager(Some(Arc::downgrade(&manager)));
    }

    api_service
        .write()
        .unwrap()
        .replace(Arc::new(instance.get_api_rpc_service()));
    
    // ... 其余代码 ...
}
```

#### 步骤3：在数据发送路径中集成带宽限制

文件：`easytier/src/peers/peer_map.rs`

在`send_msg_directly`方法中添加带宽限制检查：

```rust
pub async fn send_msg_directly(&self, msg: ZCPacket, dst_peer_id: PeerId) -> Result<(), Error> {
    // 检查流量策略 - 带宽限制
    if let Some(manager) = self.global_ctx.get_flow_policy_manager() {
        if let Some(limiter) = manager.should_limit_bandwidth() {
            let packet_size = msg.buf_len() as u64;
            // 等待令牌桶允许发送
            limiter.consume(packet_size).await;
            tracing::trace!("Bandwidth limited: consumed {} bytes", packet_size);
        }
    }
    
    if dst_peer_id == self.my_peer_id {
        let packet_send = self.packet_send.clone();
        tokio::spawn(async move {
            let ret = packet_send
                .send(msg)
                .await
                .with_context(|| "send msg to self failed");
            if ret.is_err() {
                tracing::error!("send msg to self failed: {:?}", ret);
            }
        });
        return Ok(());
    }

    match self.get_peer_by_id(dst_peer_id) {
        Some(peer) => {
            peer.send_msg(msg).await?;
        }
        None => {
            tracing::error!("no peer for dst_peer_id: {}", dst_peer_id);
            return Err(Error::RouteError(Some(format!(
                "peer map sengmsg directly no connected dst_peer_id: {}",
                dst_peer_id
            ))));
        }
    }

    Ok(())
}
```

#### 步骤4：在中转逻辑中集成禁用中转检查

文件：`easytier/src/peers/peer_manager.rs`

在`send_msg_internal`方法中添加中转检查：

```rust
async fn send_msg_internal(
    peers: &Arc<PeerMap>,
    foreign_network_client: &Arc<ForeignNetworkClient>,
    msg: ZCPacket,
    dst_peer_id: PeerId,
    global_ctx: &ArcGlobalCtx,  // 添加global_ctx参数
) -> Result<(), Error> {
    // 检查是否禁用中转
    if let Some(manager) = global_ctx.get_flow_policy_manager() {
        if manager.should_disable_relay() {
            // 检查目标是否是直连peer
            if !peers.has_peer(dst_peer_id) {
                tracing::warn!(
                    ?dst_peer_id,
                    "Relay disabled by flow policy, dropping packet"
                );
                return Err(Error::RouteError(Some(
                    "Relay disabled by flow policy".to_string()
                )));
            }
        }
    }
    
    let policy =
        Self::get_next_hop_policy(msg.peer_manager_header().unwrap().is_latency_first());

    if let Some(gateway) = peers.get_gateway_peer_id(dst_peer_id, policy.clone()).await {
        if peers.has_peer(gateway) {
            peers.send_msg_directly(msg, gateway).await
        } else if foreign_network_client.has_next_hop(gateway) {
            foreign_network_client.send_msg(msg, gateway).await
        } else {
            tracing::warn!(
                ?gateway,
                ?dst_peer_id,
                "cannot send msg to peer through gateway"
            );
            Err(Error::RouteError(None))
        }
    } else if foreign_network_client.has_next_hop(dst_peer_id) {
        foreign_network_client.send_msg(msg, dst_peer_id).await
    } else {
        tracing::debug!(?dst_peer_id, "no gateway for peer");
        Err(Error::RouteError(None))
    }
}
```

#### 步骤5：在公共转发逻辑中集成禁用公共转发检查

文件：`easytier/src/peers/foreign_network_manager.rs`

在处理外部网络数据包的地方添加检查：

```rust
// 在ForeignNetworkManager的数据包处理逻辑中
async fn handle_foreign_packet(
    &self,
    packet: ZCPacket,
    network_name: &str,
) -> Result<(), Error> {
    // 检查是否禁用公共转发
    if let Some(manager) = self.global_ctx.get_flow_policy_manager() {
        if manager.should_disable_public_forward() {
            tracing::warn!(
                ?network_name,
                "Public forward disabled by flow policy, dropping packet"
            );
            return Err(Error::Unknown);
        }
    }
    
    // ... 原有的转发逻辑 ...
}
```

### 方案2：通过Instance传递

如果不想修改GlobalCtx，可以在需要的地方直接传递Instance或FlowPolicyManager的引用。

#### 在PeerManager中添加字段

```rust
pub struct PeerManager {
    // ... 现有字段 ...
    
    flow_policy_manager: RwLock<Option<Weak<FlowPolicyManager>>>,
}

impl PeerManager {
    pub fn set_flow_policy_manager(&self, manager: Option<Weak<FlowPolicyManager>>) {
        *self.flow_policy_manager.write().await = manager;
    }
    
    pub fn get_flow_policy_manager(&self) -> Option<Arc<FlowPolicyManager>> {
        self.flow_policy_manager
            .read()
            .await
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }
}
```

## 使用示例

### 通过API设置流量策略

```rust
// 在RPC服务中处理流量策略配置
async fn set_flow_policy(
    &self,
    config: FlowPolicyConfig,
) -> Result<(), Error> {
    let instance = self.get_instance()?;
    
    // 创建或更新流量策略管理器
    if let Some(manager) = instance.get_flow_policy_manager() {
        manager.update_config(Some(config)).await;
    } else {
        let manager = FlowPolicyManager::new(
            Some(config),
            instance.get_global_ctx().stats_manager().clone(),
            instance.get_global_ctx().network.network_name.clone(),
        );
        instance.set_flow_policy_manager(Some(manager.clone()));
        
        // 设置到GlobalCtx
        instance.get_global_ctx()
            .set_flow_policy_manager(Some(Arc::downgrade(&manager)));
    }
    
    Ok(())
}
```

### 查询流量统计

```rust
async fn get_traffic_stats(&self) -> Result<TrafficStats, Error> {
    let instance = self.get_instance()?;
    
    if let Some(manager) = instance.get_flow_policy_manager() {
        Ok(manager.get_traffic_stats().await)
    } else {
        Err(Error::Unknown)
    }
}
```

### 手动重置流量

```rust
async fn reset_traffic(&self) -> Result<(), Error> {
    let instance = self.get_instance()?;
    
    if let Some(manager) = instance.get_flow_policy_manager() {
        manager.reset_traffic_stats().await;
        Ok(())
    } else {
        Err(Error::Unknown)
    }
}
```

## 测试建议

### 1. 带宽限制测试

```rust
#[tokio::test]
async fn test_bandwidth_limit() {
    // 创建测试实例
    let instance = create_test_instance().await;
    
    // 设置10Mbps带宽限制
    let config = FlowPolicyConfig {
        rules: vec![FlowPolicyRule {
            traffic_threshold_gb: 0.0,  // 立即生效
            action: FlowPolicyAction::LimitBandwidth as i32,
            bandwidth_limit_mbps: Some(10.0),
        }],
        monthly_reset_day: 1,
    };
    
    let manager = FlowPolicyManager::new(
        Some(config),
        instance.get_global_ctx().stats_manager().clone(),
        "test".to_string(),
    );
    
    instance.set_flow_policy_manager(Some(manager));
    
    // 发送大量数据并测量速度
    let start = std::time::Instant::now();
    send_test_data(&instance, 10 * 1024 * 1024).await; // 10MB
    let duration = start.elapsed();
    
    // 验证速度接近10Mbps
    let speed_mbps = (10.0 * 8.0) / duration.as_secs_f64();
    assert!(speed_mbps < 12.0 && speed_mbps > 8.0);
}
```

### 2. 中转禁用测试

```rust
#[tokio::test]
async fn test_relay_disabled() {
    let instance = create_test_instance().await;
    
    // 设置禁用中转策略
    let config = FlowPolicyConfig {
        rules: vec![FlowPolicyRule {
            traffic_threshold_gb: 0.0,
            action: FlowPolicyAction::DisableRelay as i32,
            bandwidth_limit_mbps: None,
        }],
        monthly_reset_day: 1,
    };
    
    let manager = FlowPolicyManager::new(
        Some(config),
        instance.get_global_ctx().stats_manager().clone(),
        "test".to_string(),
    );
    
    instance.set_flow_policy_manager(Some(manager));
    
    // 尝试通过中转发送数据
    let result = send_via_relay(&instance, remote_peer_id).await;
    
    // 验证中转被拒绝
    assert!(result.is_err());
}
```

### 3. 流量阈值测试

```rust
#[tokio::test]
async fn test_traffic_threshold() {
    let instance = create_test_instance().await;
    
    // 设置1GB阈值后限制带宽
    let config = FlowPolicyConfig {
        rules: vec![FlowPolicyRule {
            traffic_threshold_gb: 1.0,
            action: FlowPolicyAction::LimitBandwidth as i32,
            bandwidth_limit_mbps: Some(1.0),
        }],
        monthly_reset_day: 1,
    };
    
    let manager = FlowPolicyManager::new(
        Some(config),
        instance.get_global_ctx().stats_manager().clone(),
        "test".to_string(),
    );
    
    instance.set_flow_policy_manager(Some(manager.clone()));
    
    // 发送0.5GB数据，不应该被限制
    send_test_data(&instance, 512 * 1024 * 1024).await;
    assert!(manager.should_limit_bandwidth().is_none());
    
    // 再发送0.6GB数据，总共超过1GB，应该被限制
    send_test_data(&instance, 600 * 1024 * 1024).await;
    assert!(manager.should_limit_bandwidth().is_some());
}
```

## 性能考虑

1. **带宽限制开销**：Token Bucket算法非常高效，每次consume操作的开销很小
2. **策略检查开销**：使用DashMap进行O(1)查找，开销可忽略
3. **流量统计更新**：每10秒更新一次，不影响数据路径性能
4. **弱引用使用**：避免循环引用，确保正确的生命周期管理

## 注意事项

1. **线程安全**：所有管理器都使用Arc和RwLock保证线程安全
2. **生命周期**：使用Weak引用避免循环依赖
3. **错误处理**：策略检查失败时应该记录日志但不应该崩溃
4. **配置更新**：支持运行时动态更新策略配置
5. **向后兼容**：如果没有设置流量策略，系统应该正常工作

## 总结

流量策略的集成需要在多个关键位置添加检查逻辑：

1. ✅ **带宽限制**：在`send_msg_directly`中检查并应用
2. ✅ **禁用中转**：在`send_msg_internal`中检查
3. ✅ **禁用公共转发**：在`ForeignNetworkManager`中检查

推荐使用方案1（通过GlobalCtx传递），因为它提供了全局访问能力，且不需要修改太多现有代码。
