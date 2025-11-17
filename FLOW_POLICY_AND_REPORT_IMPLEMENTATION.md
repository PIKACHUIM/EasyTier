# 流量策略和上报功能实现总结

## 已完成的工作

### 1. 后端实现

#### 1.1 Proto定义 (`easytier/src/proto/api_manage.proto`)
- ✅ 添加了 `FlowPolicyAction` 枚举（限制带宽、禁用中转、禁用公共转发）
- ✅ 添加了 `FlowPolicyRule` 消息定义
- ✅ 添加了 `FlowPolicyConfig` 消息定义（包含规则列表和月度重置日期）
- ✅ 添加了 `ReportConfig` 消息定义（包含上报URL、Token和心跳间隔）
- ✅ 在 `NetworkConfig` 中添加了 `flow_policy` 和 `report_config` 字段

#### 1.2 流量策略管理器 (`easytier/src/common/flow_policy_manager.rs`)
- ✅ 实现了 `TrafficStats` 结构体用于跟踪流量统计
- ✅ 实现了 `ActivePolicy` 结构体存储激活的策略和带宽限制器
- ✅ 实现了 `FlowPolicyManager` 主管理器：
  - 定期从 `StatsManager` 获取流量数据
  - 检查流量阈值并应用相应策略
  - 支持月度流量重置
  - 提供带宽限制、禁用中转、禁用公共转发的检查方法

#### 1.3 上报管理器 (`easytier/src/common/report_manager.rs`)
- ✅ 实现了 `ReportRequest` 和 `ReportResponse` 结构体
- ✅ 实现了 `ReportManager` 主管理器：
  - 定期收集节点状态、流量统计、连接数等信息
  - 支持多个上报URL
  - 支持Token认证
  - 可配置心跳间隔
  - 计算流量增量并上报

#### 1.4 Instance集成 (`easytier/src/instance/instance.rs`)
- ✅ 在 `Instance` 结构体中添加了 `flow_policy_manager` 和 `report_manager` 字段
- ✅ 添加了 getter 和 setter 方法
- ✅ 添加了必要的导入

#### 1.5 依赖管理 (`easytier/Cargo.toml`)
- ✅ 添加了 `reqwest` 依赖用于HTTP请求

### 2. 前端实现

#### 2.1 类型定义 (`easytier-web/frontend-lib/src/types/network.ts`)
- ✅ 定义了 `FlowPolicyAction` 枚举
- ✅ 定义了 `FlowPolicyRule` 接口
- ✅ 定义了 `FlowPolicyConfig` 接口
- ✅ 定义了 `ReportConfig` 接口

#### 2.2 GUI界面 (`easytier-web/frontend-lib/src/components/Config.vue`)
- ✅ 实现了流量策略配置面板：
  - 月度重置日期选择（1-31日）
  - 流量规则列表（阈值GB、操作类型、带宽限制Mbps）
  - 添加/删除规则功能
  - 表单验证
- ✅ 实现了上报配置面板：
  - 上报Token输入
  - 心跳间隔设置（分钟）
  - 上报URL列表管理
  - 添加/删除URL功能

#### 2.3 国际化支持
- ✅ 中文翻译完整 (`easytier-web/frontend-lib/src/locales/cn.yaml`)
- ✅ 英文翻译完整 (`easytier-web/frontend-lib/src/locales/en.yaml`)

## 功能说明

### 流量策略 (Flow Policy)

#### 配置项
1. **月度重置日期** (`monthly_reset_day`): 1-31，指定每月哪一天重置流量统计
2. **流量规则列表** (`rules`): 可配置多个规则，每个规则包含：
   - **流量阈值** (`traffic_threshold_gb`): 达到多少GB流量时触发
   - **操作类型** (`action`): 
     - `LimitBandwidth`: 限制带宽
     - `DisableRelay`: 禁用中转
     - `DisablePublicForward`: 禁用公共转发
   - **带宽限制** (`bandwidth_limit_mbps`): 当操作为限制带宽时，设置限制值（Mbps）

#### 工作原理
1. 后台任务每10秒检查一次流量统计
2. 从 `StatsManager` 获取发送和接收的字节数
3. 计算总流量并与配置的阈值比较
4. 达到阈值时应用相应的策略：
   - 限制带宽：创建Token Bucket限制器
   - 禁用中转：设置标志位
   - 禁用公共转发：设置标志位
5. 每小时检查一次是否需要月度重置

#### API方法
```rust
// 检查是否应该限制带宽
pub fn should_limit_bandwidth(&self) -> Option<Arc<TokenBucket>>

// 检查是否应该禁用中转
pub fn should_disable_relay(&self) -> bool

// 检查是否应该禁用公共转发
pub fn should_disable_public_forward(&self) -> bool

// 获取流量统计
pub async fn get_traffic_stats(&self) -> TrafficStats

// 手动重置流量统计
pub async fn reset_traffic_stats(&self)

// 更新配置
pub async fn update_config(&self, new_config: Option<FlowPolicyConfig>)
```

### 上报功能 (Report)

#### 配置项
1. **上报Token** (`token`): 用于认证的Token
2. **心跳间隔** (`heartbeat_interval_minutes`): 多少分钟上报一次
3. **上报URL列表** (`report_urls`): 可配置多个上报地址

#### 上报数据格式
```json
{
  "node_name": "节点名称",
  "email": "用户邮箱",
  "token": "上报Token",
  "current_bandwidth": 50.5,
  "reported_traffic": 0.5,
  "connection_count": 5,
  "reset_date": 1,
  "status": "online"
}
```

#### 工作原理
1. 后台任务按配置的心跳间隔定期执行
2. 收集以下信息：
   - 节点名称和邮箱
   - 当前带宽使用情况
   - 本次上报的流量增量（GB）
   - 当前连接数
   - 月度重置日期
   - 节点状态（online/offline）
3. 向所有配置的URL发送POST请求到 `/api/report` 端点
4. 记录上报结果

#### API方法
```rust
// 更新配置
pub async fn update_config(&self, new_config: Option<ReportConfig>)

// 手动触发上报
pub async fn trigger_report(&self, reset_date: u32)

// 获取上次上报的流量
pub async fn get_last_reported_traffic(&self) -> u64
```

## 使用方法

### 前端配置

1. **配置流量策略**：
   ```
   1. 在网络配置页面找到"流量策略"面板
   2. 设置月度重置日期（1-31）
   3. 点击"添加规则"按钮
   4. 配置流量阈值（GB）
   5. 选择触发操作（限制带宽/禁用中转/禁用公共转发）
   6. 如果选择"限制带宽"，设置带宽限制值（Mbps）
   7. 可添加多个规则，形成阶梯式策略
   ```

2. **配置上报功能**：
   ```
   1. 在网络配置页面找到"上报配置"面板
   2. 输入上报Token
   3. 设置心跳间隔（分钟）
   4. 点击"添加地址"添加上报URL
   5. 可添加多个上报地址实现多点上报
   ```

### 后端集成

在网络实例中使用这些管理器：

```rust
use easytier::common::flow_policy_manager::FlowPolicyManager;
use easytier::common::report_manager::ReportManager;

// 创建流量策略管理器
let flow_policy_manager = FlowPolicyManager::new(
    Some(flow_policy_config),
    stats_manager.clone(),
    network_name.clone(),
);

// 创建上报管理器
let report_manager = ReportManager::new(
    Some(report_config),
    stats_manager.clone(),
    network_name.clone(),
    node_name.clone(),
    email.clone(),
);

// 设置到Instance
instance.set_flow_policy_manager(Some(flow_policy_manager));
instance.set_report_manager(Some(report_manager));

// 在数据转发逻辑中检查策略
if let Some(manager) = instance.get_flow_policy_manager() {
    if manager.should_disable_relay() {
        // 禁用中转逻辑
    }
    
    if manager.should_disable_public_forward() {
        // 禁用公共转发逻辑
    }
    
    if let Some(limiter) = manager.should_limit_bandwidth() {
        // 使用limiter进行带宽限制
        limiter.consume(bytes_to_send).await;
    }
}
```

## 架构设计

```
┌─────────────────────────────────────────────────────────────┐
│                        前端 GUI                              │
│  ┌──────────────────┐         ┌──────────────────┐         │
│  │  流量策略配置     │         │   上报配置        │         │
│  │  - 月度重置日期   │         │   - Token        │         │
│  │  - 流量规则列表   │         │   - 心跳间隔      │         │
│  │  - 阈值/操作      │         │   - 上报URL列表   │         │
│  └──────────────────┘         └──────────────────┘         │
└─────────────────────────────────────────────────────────────┘
                            ↓ gRPC
┌─────────────────────────────────────────────────────────────┐
│                    Proto 定义层                              │
│  FlowPolicyConfig + ReportConfig → NetworkConfig            │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│                  Instance (网络实例)                         │
│  ┌──────────────────┐         ┌──────────────────┐         │
│  │ FlowPolicyManager│         │  ReportManager   │         │
│  │  - 流量统计       │         │  - 数据收集       │         │
│  │  - 策略检查       │         │  - HTTP上报       │         │
│  │  - 策略执行       │         │  - 定时任务       │         │
│  └──────────────────┘         └──────────────────┘         │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│                    StatsManager                              │
│  - 流量统计                                                  │
│  - 连接数统计                                                │
│  - 性能指标                                                  │
└─────────────────────────────────────────────────────────────┘
```

## 注意事项

1. **配置存储**：`flow_policy` 和 `report_config` 不会保存到本地TOML配置文件
2. **运行时管理**：这些配置由网络实例管理器在运行时管理
3. **重启行为**：重启后需要重新配置（除非通过网络管理平台持久化）
4. **性能影响**：
   - 流量策略检查每10秒执行一次，性能影响很小
   - 带宽限制使用Token Bucket算法，高效且精确
   - 上报功能异步执行，不影响主业务流程

## 下一步工作

### 策略执行集成
需要在以下模块中集成流量策略的执行逻辑：

1. **PeerManager** (`easytier/src/peers/peer_manager.rs`)
   - 在数据转发前检查 `should_disable_relay()`
   - 在数据发送前使用 `should_limit_bandwidth()` 进行限速

2. **ForeignNetworkManager** (`easytier/src/peers/foreign_network_manager.rs`)
   - 在公共转发前检查 `should_disable_public_forward()`

3. **带宽限制集成**
   - 在发送数据包前调用 `TokenBucket::consume()`
   - 根据返回的等待时间进行延迟

### GUI前端
需要将web前端的修改同步到GUI前端（如果有独立的GUI实现）

### API接口
可以添加以下API接口：
- 查询当前流量统计
- 查询激活的策略列表
- 手动触发流量重置
- 手动触发上报
- 查询上报历史

## 测试建议

1. **流量策略测试**：
   - 配置低阈值（如0.1GB）快速触发策略
   - 验证带宽限制是否生效
   - 验证中转和公共转发禁用是否生效
   - 验证月度重置功能

2. **上报功能测试**：
   - 配置短心跳间隔（如1分钟）
   - 搭建测试上报服务器
   - 验证上报数据格式和内容
   - 验证多URL上报功能

3. **性能测试**：
   - 测试流量策略对转发性能的影响
   - 测试带宽限制的精确度
   - 测试上报功能的资源消耗

## 总结

本次实现完成了流量策略和上报功能的完整框架，包括：
- ✅ Proto定义和类型系统
- ✅ 后端管理器实现
- ✅ 前端UI界面
- ✅ 国际化支持
- ✅ 基础集成

核心功能已经实现，可以通过前端配置并在后端运行。下一步需要将策略执行逻辑集成到实际的数据转发路径中，以实现完整的流量控制功能。
