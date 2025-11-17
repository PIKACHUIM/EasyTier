use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::time::interval;

use crate::common::scoped_task::ScopedTask;
use crate::common::stats_manager::{LabelSet, MetricName, StatsManager};
use crate::common::token_bucket::TokenBucket;
use crate::proto::api::manage::{FlowPolicyAction, FlowPolicyConfig, FlowPolicyRule};

/// 流量统计数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficStats {
    /// 总发送字节数
    pub tx_bytes: u64,
    /// 总接收字节数
    pub rx_bytes: u64,
    /// 总字节数（发送+接收）
    pub total_bytes: u64,
    /// 上次重置时间（Unix时间戳）
    pub last_reset_time: u64,
}

impl Default for TrafficStats {
    fn default() -> Self {
        Self {
            tx_bytes: 0,
            rx_bytes: 0,
            total_bytes: 0,
            last_reset_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
}

impl TrafficStats {
    /// 重置统计数据
    pub fn reset(&mut self) {
        self.tx_bytes = 0;
        self.rx_bytes = 0;
        self.total_bytes = 0;
        self.last_reset_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// 添加流量数据
    pub fn add_traffic(&mut self, tx_bytes: u64, rx_bytes: u64) {
        self.tx_bytes += tx_bytes;
        self.rx_bytes += rx_bytes;
        self.total_bytes = self.tx_bytes + self.rx_bytes;
    }

    /// 获取总流量（GB）
    pub fn total_gb(&self) -> f64 {
        self.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

/// 当前激活的策略状态
#[derive(Debug, Clone)]
pub struct ActivePolicy {
    /// 策略规则
    pub rule: FlowPolicyRule,
    /// 带宽限制器（如果策略是限制带宽）
    pub bandwidth_limiter: Option<Arc<TokenBucket>>,
}

/// 流量策略管理器
pub struct FlowPolicyManager {
    /// 流量策略配置
    config: Arc<RwLock<Option<FlowPolicyConfig>>>,
    /// 流量统计数据
    traffic_stats: Arc<RwLock<TrafficStats>>,
    /// 当前激活的策略
    active_policies: Arc<DashMap<FlowPolicyAction, ActivePolicy>>,
    /// 统计管理器引用
    stats_manager: Arc<StatsManager>,
    /// 网络名称
    network_name: String,
    /// 后台任务
    background_task: ScopedTask<()>,
}

impl FlowPolicyManager {
    /// 创建新的流量策略管理器
    pub fn new(
        config: Option<FlowPolicyConfig>,
        stats_manager: Arc<StatsManager>,
        network_name: String,
    ) -> Arc<Self> {
        let config_arc = Arc::new(RwLock::new(config));
        let traffic_stats = Arc::new(RwLock::new(TrafficStats::default()));
        let active_policies = Arc::new(DashMap::new());

        // 创建后台任务的弱引用结构
        let config_weak = Arc::downgrade(&config_arc);
        let traffic_stats_weak = Arc::downgrade(&traffic_stats);
        let active_policies_weak = Arc::downgrade(&active_policies);
        let stats_manager_weak = Arc::downgrade(&stats_manager);
        let network_name_clone = network_name.clone();

        let background_task = tokio::spawn(async move {
            Self::run_background_tasks_static(
                config_weak,
                traffic_stats_weak,
                active_policies_weak,
                stats_manager_weak,
                network_name_clone,
            ).await;
        });

        Arc::new(Self {
            config: config_arc,
            traffic_stats,
            active_policies,
            stats_manager,
            network_name,
            background_task: background_task.into(),
        })
    }

    /// 静态后台任务运行方法
    async fn run_background_tasks_static(
        config: std::sync::Weak<RwLock<Option<FlowPolicyConfig>>>,
        traffic_stats: std::sync::Weak<RwLock<TrafficStats>>,
        active_policies: std::sync::Weak<DashMap<FlowPolicyAction, ActivePolicy>>,
        stats_manager: std::sync::Weak<StatsManager>,
        network_name: String,
    ) {
        let mut check_interval = interval(Duration::from_secs(10));
        let mut reset_check_interval = interval(Duration::from_secs(3600));

        loop {
            tokio::select! {
                _ = check_interval.tick() => {
                    let Some(stats_mgr) = stats_manager.upgrade() else { break; };
                    let Some(t_stats) = traffic_stats.upgrade() else { break; };
                    let Some(cfg) = config.upgrade() else { break; };
                    let Some(policies) = active_policies.upgrade() else { break; };

                    Self::update_traffic_stats_static(&stats_mgr, &t_stats, &network_name).await;
                    Self::check_and_apply_policies_static(&cfg, &t_stats, &policies).await;
                }
                _ = reset_check_interval.tick() => {
                    let Some(cfg) = config.upgrade() else { break; };
                    let Some(t_stats) = traffic_stats.upgrade() else { break; };
                    let Some(policies) = active_policies.upgrade() else { break; };

                    Self::check_monthly_reset_static(&cfg, &t_stats, &policies).await;
                }
            }
        }
    }

    /// 静态方法：更新流量统计
    async fn update_traffic_stats_static(
        stats_manager: &StatsManager,
        traffic_stats: &RwLock<TrafficStats>,
        network_name: &str,
    ) {
        let label_set = LabelSet::new()
            .with_label("network_name", network_name.to_string());

        let tx_bytes = stats_manager
            .get_metric(MetricName::TrafficBytesTx, &label_set)
            .map(|m| m.value)
            .unwrap_or(0);

        let rx_bytes = stats_manager
            .get_metric(MetricName::TrafficBytesRx, &label_set)
            .map(|m| m.value)
            .unwrap_or(0);

        let mut stats = traffic_stats.write().await;
        stats.tx_bytes = tx_bytes;
        stats.rx_bytes = rx_bytes;
        stats.total_bytes = tx_bytes + rx_bytes;
    }

    /// 静态方法：检查并应用策略
    async fn check_and_apply_policies_static(
        config: &RwLock<Option<FlowPolicyConfig>>,
        traffic_stats: &RwLock<TrafficStats>,
        active_policies: &DashMap<FlowPolicyAction, ActivePolicy>,
    ) {
        let cfg = config.read().await;
        let Some(ref policy_config) = *cfg else {
            return;
        };

        let stats = traffic_stats.read().await;
        let total_gb = stats.total_gb();

        // 清除所有当前激活的策略
        active_policies.clear();

        // 检查每个规则
        for rule in &policy_config.rules {
            if total_gb >= rule.traffic_threshold_gb {
                Self::apply_policy_static(rule.clone(), active_policies).await;
            }
        }
    }

    /// 静态方法：应用策略
async fn apply_policy_static(
        rule: FlowPolicyRule,
        active_policies: &DashMap<FlowPolicyAction, ActivePolicy>,
    ) {
        let action = FlowPolicyAction::try_from(rule.action).unwrap_or(FlowPolicyAction::LimitBandwidth);

        let bandwidth_limiter = if action == FlowPolicyAction::LimitBandwidth {
            if let Some(bandwidth_mbps) = rule.bandwidth_limit_mbps {
                let bps = (bandwidth_mbps * 1024.0 * 1024.0 / 8.0) as u64;
                let capacity = bps * 2;
                let refill_interval = Duration::from_millis(100);
                Some(TokenBucket::new(capacity, bps, refill_interval))
            } else {
                None
            }
        } else {
            None
        };

        let active_policy = ActivePolicy {
            rule: rule.clone(),
            bandwidth_limiter,
        };

        active_policies.insert(action, active_policy);

        tracing::info!(
            "Applied flow policy: action={:?}, threshold={}GB",
            action,
            rule.traffic_threshold_gb
        );
    }

    /// 静态方法：检查是否需要每月重置
    async fn check_monthly_reset_static(
        config: &RwLock<Option<FlowPolicyConfig>>,
        traffic_stats: &RwLock<TrafficStats>,
        active_policies: &DashMap<FlowPolicyAction, ActivePolicy>,
    ) {
        let cfg = config.read().await;
        let Some(ref policy_config) = *cfg else {
            return;
        };

        let reset_day = policy_config.monthly_reset_day;
        if reset_day < 1 || reset_day > 31 {
            return;
        }

        let stats = traffic_stats.read().await;
        let last_reset = stats.last_reset_time;
        drop(stats);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // 简单检查：如果距离上次重置超过30天，则重置
        // 这是一个简化的实现，实际应该检查具体的日期
        let days_since_reset = (now - last_reset) / (24 * 3600);
        if days_since_reset >= 30 {
            let mut stats = traffic_stats.write().await;
            stats.reset();
            drop(stats);

            active_policies.clear();

            tracing::info!("Monthly traffic stats reset (30+ days since last reset)");
        }
    }



    /// 重置流量统计
    pub async fn reset_traffic_stats(&self) {
        let mut stats = self.traffic_stats.write().await;
        stats.reset();

        // 清除激活的策略
        self.active_policies.clear();

        tracing::info!("Traffic stats reset");
    }

    /// 获取流量统计
    pub async fn get_traffic_stats(&self) -> TrafficStats {
        self.traffic_stats.read().await.clone()
    }

    /// 更新配置
    pub async fn update_config(&self, new_config: Option<FlowPolicyConfig>) {
        let mut config = self.config.write().await;
        *config = new_config;
        drop(config);

        // 立即检查并应用策略
        Self::check_and_apply_policies_static(&self.config, &self.traffic_stats, &self.active_policies).await;
    }

    /// 检查是否应该限制带宽
    pub fn should_limit_bandwidth(&self) -> Option<Arc<TokenBucket>> {
        self.active_policies
            .get(&FlowPolicyAction::LimitBandwidth)
            .and_then(|policy| policy.bandwidth_limiter.clone())
    }

    /// 检查是否应该禁用中转
    pub fn should_disable_relay(&self) -> bool {
        self.active_policies.contains_key(&FlowPolicyAction::DisableRelay)
    }

    /// 检查是否应该禁用公共转发
    pub fn should_disable_public_forward(&self) -> bool {
        self.active_policies.contains_key(&FlowPolicyAction::DisablePublicForward)
    }

    /// 获取当前激活的策略列表
    pub fn get_active_policies(&self) -> Vec<(FlowPolicyAction, FlowPolicyRule)> {
        self.active_policies
            .iter()
            .map(|entry| (*entry.key(), entry.value().rule.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_traffic_stats() {
        let mut stats = TrafficStats::default();
        assert_eq!(stats.total_bytes, 0);

        stats.add_traffic(1024 * 1024 * 1024, 1024 * 1024 * 1024); // 1GB + 1GB
        assert_eq!(stats.total_gb(), 2.0);

        stats.reset();
        assert_eq!(stats.total_bytes, 0);
    }

    #[tokio::test]
    async fn test_flow_policy_manager_creation() {
        let stats_manager = Arc::new(StatsManager::new());
        let config = Some(FlowPolicyConfig {
            rules: vec![],
            monthly_reset_day: 1,
        });

        let manager = FlowPolicyManager::new(
            config,
            stats_manager,
            "test_network".to_string(),
        );

        let stats = manager.get_traffic_stats().await;
        assert_eq!(stats.total_bytes, 0);
    }

    #[tokio::test]
    async fn test_policy_application() {
        let stats_manager = Arc::new(StatsManager::new());
        
        let rule = FlowPolicyRule {
            traffic_threshold_gb: 1.0,
            action: FlowPolicyAction::LimitBandwidth as i32,
            bandwidth_limit_mbps: Some(10.0),
        };

        let config = Some(FlowPolicyConfig {
            rules: vec![rule],
            monthly_reset_day: 1,
        });

        let manager = FlowPolicyManager::new(
            config,
            stats_manager,
            "test_network".to_string(),
        );

        // 初始状态不应该有激活的策略
        assert!(!manager.should_disable_relay());
        assert!(!manager.should_disable_public_forward());
    }
}
