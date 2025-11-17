# 流量策略集成 - 快速开始指南

本指南提供最简化的步骤来完成流量策略和上报功能的集成。

## 🚀 快速集成步骤

### 步骤1：添加PolicyContainer（5分钟）

创建新文件 `easytier/src/common/policy_container.rs`：

```rust
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use crate::common::flow_policy_manager::FlowPolicyManager;
use crate::common::report_manager::ReportManager;

/// 策略容器，用于存储流量策略管理器和上报管理器的弱引用
pub struct PolicyContainer {
    flow_policy_manager: RwLock<Option<Weak<FlowPolicyManager>>>,
    report_manager: RwLock<Option<Weak<ReportManager>>>,
}

impl PolicyContainer {
    pub fn new() -> Self {
        Self {
            flow_policy_manager: RwLock::new(None),
            report_manager: RwLock::new(None),
        }
    }
    
    pub async fn set_flow_policy_manager(&self, manager: Option<Arc<FlowPolicyManager>>) {
        *self.flow_policy_manager.write().await = manager.map(|m| Arc::downgrade(&m));
    }
    
    pub async fn get_flow_policy_manager(&self) -> Option<Arc<FlowPolicyManager>> {
        self.flow_policy_manager
            .read()
            .await
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }
    
    pub async fn set_report_manager(&self, manager: Option<Arc<ReportManager>>) {
        *self.report_manager.write().await = manager.map(|m| Arc::downgrade(&m));
    }
    
    pub async fn get_report_manager(&self) -> Option<Arc<ReportManager>> {
        self.report_manager
            .read()
            .await
            .as_ref()
            .and_then(|weak| weak.upgrade())
    }
}

impl Default for PolicyContainer {
    fn default() -> Self {
        Self::new()
    }
}
```

在 `easytier/src/common/mod.rs` 中注册模块：

```rust
pub mod policy_container;
```

### 步骤2：修改GlobalCtx（10分钟）

在 `easytier/src/common/global_ctx.rs` 中：

**2.1 添加导入：**

```rust
use crate::common::policy_container::PolicyContainer;
```

**2.2 在GlobalCtx结构体中添加字段：**

```rust
pub struct GlobalCtx {
    // ... 现有字段 ...
    
    policy_container: Arc<PolicyContainer>,
}
```

**2.3 在new方法中初始化：**

```rust
impl GlobalCtx {
    pub fn new(config_fs: impl ConfigLoader + 'static) -> Self {
        // ... 现有初始化代码 ...
        
        GlobalCtx {
            // ... 现有字段初始化 ...
            policy_container: Arc::new(PolicyContainer::new()),
        }
    }
}
```

**2.4 添加访问方法：**

```rust
impl GlobalCtx {
    // ... 现有方法 ...
    
    pub fn policy_container(&self) -> &Arc<PolicyContainer> {
        &self.policy_container
    }
}
```

### 步骤3：集成带宽限制（15分钟）

在 `easytier/src/peers/peer_map.rs` 的 `send_msg_directly` 方法开头添加：

```rust
pub async fn send_msg_directly(&self, msg: ZCPacket, dst_peer_id: PeerId) -> Result<(), Error> {
    // 检查流量策略 - 带宽限制
    if let Some(manager) = self.global_ctx.policy_container().get_flow_policy_manager().await {
        if let Some(limiter) = manager.should_limit_bandwidth() {
            let packet_size = msg.buf_len() as u64;
            limiter.consume(packet_size).await;
            tracing::trace!("Bandwidth limited: consumed {} bytes", packet_size);
        }
    }
    
    // ... 原有代码保持不变 ...
}
```

### 步骤4：集成中转禁用（15分钟）

在 `easytier/src/peers/peer_manager.rs` 的 `send_msg_internal` 方法开头添加：

找到这个方法：

```rust
async fn send_msg_internal(
    peers: &Arc<PeerMap>,
    foreign_network_client: &Arc<ForeignNetworkClient>,
    msg: ZCPacket,
    dst_peer_id: PeerId,
) -> Result<(), Error> {
```

在方法开头添加：

```rust
    // 检查是否禁用中转
    if let Some(manager) = peers.global_ctx.policy_container().get_flow_policy_manager().await {
        if manager.should_disable_relay() {
            // 如果目标不是直连peer，拒绝转发
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
    
    // ... 原有代码保持不变 ...
```

### 步骤5：集成公共转发禁用（15分钟）

在 `easytier/src/peers/foreign_network_manager.rs` 中找到数据包转发的方法。

搜索 `send_msg_to_peer` 方法，在开头添加：

```rust
pub async fn send_msg_to_peer(
    &self,
    network_name: &str,
    peer_id: PeerId,
    msg: ZCPacket,
) -> Result<(), Error> {
    // 检查是否禁用公共转发
    if let Some(manager) = self.global_ctx.policy_container().get_flow_policy_manager().await {
        if manager.should_disable_public_forward() {
            tracing::warn!(
                ?network_name,
                ?peer_id,
                "Public forward disabled by flow policy, dropping packet"
            );
            return Err(Error::Unknown);
        }
    }
    
    // ... 原有代码保持不变 ...
}
```

### 步骤6：在Launcher中设置管理器（10分钟）

在 `easytier/src/launcher.rs` 的 `easytier_routine` 方法中，找到 `instance.run().await?;` 这一行，在它之后添加：

```rust
    instance.run().await?;

    // 设置流量策略管理器和上报管理器到GlobalCtx
    let global_ctx = instance.get_global_ctx();
    
    if let Some(manager) = instance.get_flow_policy_manager() {
        global_ctx.policy_container()
            .set_flow_policy_manager(Some(manager))
            .await;
        tracing::info!("Flow policy manager initialized");
    }
    
    if let Some(manager) = instance.get_report_manager() {
        global_ctx.policy_container()
            .set_report_manager(Some(manager))
            .await;
        tracing::info!("Report manager initialized");
    }

    api_service
        .write()
        .unwrap()
        .replace(Arc::new(instance.get_api_rpc_service()));
```

### 步骤7：编译测试（10分钟）

```bash
cd easytier
cargo build
```

如果有编译错误，根据错误信息修复。

### 步骤8：运行测试（5分钟）

```bash
cargo test --lib flow_policy
cargo test --lib report_manager
```

## 🎯 验证集成

### 手动测试

创建一个简单的测试程序来验证功能：

```rust
#[tokio::test]
async fn test_flow_policy_integration() {
    use crate::common::flow_policy_manager::FlowPolicyManager;
    use crate::proto::api::manage::{FlowPolicyConfig, FlowPolicyRule, FlowPolicyAction};
    
    // 创建测试配置
    let config = FlowPolicyConfig {
        rules: vec![FlowPolicyRule {
            traffic_threshold_gb: 0.0,  // 立即生效
            action: FlowPolicyAction::LimitBandwidth as i32,
            bandwidth_limit_mbps: Some(10.0),
        }],
        monthly_reset_day: 1,
    };
    
    // 创建管理器
    let stats_manager = Arc::new(StatsManager::new());
    let manager = FlowPolicyManager::new(
        Some(config),
        stats_manager,
        "test".to_string(),
    );
    
    // 等待策略应用
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // 验证策略已激活
    assert!(manager.should_limit_bandwidth().is_some());
    println!("✅ Flow policy integration test passed!");
}
```

## 📋 集成检查清单

完成每个步骤后打勾：

- [ ] 步骤1：创建PolicyContainer
- [ ] 步骤2：修改GlobalCtx
- [ ] 步骤3：集成带宽限制
- [ ] 步骤4：集成中转禁用
- [ ] 步骤5：集成公共转发禁用
- [ ] 步骤6：在Launcher中设置管理器
- [ ] 步骤7：编译成功
- [ ] 步骤8：测试通过

## 🐛 常见问题

### 问题1：编译错误 - 找不到PolicyContainer

**解决方案**：确保在 `mod.rs` 中添加了 `pub mod policy_container;`

### 问题2：编译错误 - 类型不匹配

**解决方案**：检查是否正确使用了 `Arc` 和 `Weak` 引用

### 问题3：运行时panic - unwrap失败

**解决方案**：使用 `if let Some(...)` 而不是 `unwrap()`

### 问题4：策略不生效

**解决方案**：
1. 检查管理器是否正确设置到GlobalCtx
2. 检查流量阈值是否设置正确
3. 查看日志确认策略是否被应用

## 💡 调试技巧

### 1. 添加日志

在关键位置添加日志：

```rust
tracing::info!("Flow policy manager set: {:?}", manager.is_some());
tracing::debug!("Checking bandwidth limit...");
tracing::warn!("Relay disabled, dropping packet");
```

### 2. 查看激活的策略

```rust
if let Some(manager) = global_ctx.policy_container().get_flow_policy_manager().await {
    let policies = manager.get_active_policies();
    tracing::info!("Active policies: {:?}", policies);
}
```

### 3. 查看流量统计

```rust
if let Some(manager) = global_ctx.policy_container().get_flow_policy_manager().await {
    let stats = manager.get_traffic_stats().await;
    tracing::info!("Traffic stats: {:?}", stats);
}
```

## 🎉 完成！

完成以上步骤后，流量策略和上报功能就已经完全集成到EasyTier中了！

### 下一步

1. **添加API接口**：在RPC服务中添加配置和查询API
2. **前端测试**：测试前端UI是否能正确配置策略
3. **性能测试**：测试带宽限制的精确度
4. **文档更新**：更新用户文档说明新功能

## 📚 相关文档

- [实现总结](FLOW_POLICY_AND_REPORT_IMPLEMENTATION.md)
- [集成指南](FLOW_POLICY_INTEGRATION_GUIDE.md)
- [当前状态](FLOW_POLICY_STATUS.md)

---

**预计总时间：约1-1.5小时**

如有问题，请参考详细的集成指南文档。
