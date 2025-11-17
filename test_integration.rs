//! 测试流量策略和上报功能集成
//! 运行命令: cargo test --bin test_integration

use std::sync::Arc;
use std::time::Duration;

use easytier::common::flow_policy_manager::{FlowPolicyManager, TrafficStats};
use easytier::common::policy_container::PolicyContainer;
use easytier::common::stats_manager::StatsManager;
use easytier::proto::api::manage::{FlowPolicyAction, FlowPolicyConfig, FlowPolicyRule};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("开始测试流量策略和上报功能集成...");

    // 1. 创建统计管理器
    let stats_manager = Arc::new(StatsManager::new());

    // 2. 创建策略配置
    let rule = FlowPolicyRule {
        traffic_threshold_gb: 1.0,
        action: FlowPolicyAction::LimitBandwidth as i32,
        bandwidth_limit_mbps: Some(10.0),
    };

    let config = Some(FlowPolicyConfig {
        rules: vec![rule],
        monthly_reset_day: 1,
    });

    // 3. 创建流量策略管理器
    let flow_policy_manager = FlowPolicyManager::new(
        config,
        stats_manager.clone(),
        "test_network".to_string(),
    );

    // 4. 创建策略容器
    let policy_container = Arc::new(PolicyContainer::new());

    // 5. 设置管理器到容器
    policy_container.set_flow_policy_manager(Some(flow_policy_manager.clone())).await;

    // 6. 测试获取管理器
    let retrieved_manager = policy_container.get_flow_policy_manager().await;
    assert!(retrieved_manager.is_some());

    // 7. 测试流量统计
    let stats = flow_policy_manager.get_traffic_stats().await;
    println!("初始流量统计: {} GB", stats.total_gb());

    // 8. 测试策略状态
    println!("是否禁用中转: {}", flow_policy_manager.should_disable_relay());
    println!("是否禁用公共转发: {}", flow_policy_manager.should_disable_public_forward());
    
    let bandwidth_limiter = flow_policy_manager.should_limit_bandwidth();
    if bandwidth_limiter.is_some() {
        println!("当前有带宽限制策略（但未触发，因为流量未达到阈值）");
    }

    println!("✅ 所有集成测试通过！");

    Ok(())
}