use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::common::scoped_task::ScopedTask;
use crate::common::stats_manager::{LabelSet, MetricName, StatsManager};
use crate::proto::api::manage::ReportConfig;

/// 上报请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRequest {
    /// 节点名称
    pub node_name: String,
    /// 用户邮箱
    pub email: String,
    /// 上报Token
    pub token: String,
    /// 当前带宽（Mbps）
    pub current_bandwidth: f64,
    /// 本次上报的流量增量（GB）
    pub reported_traffic: f64,
    /// 当前连接数
    pub connection_count: u32,
    /// 每月重置日期
    pub reset_date: u32,
    /// 当前状态
    pub status: String,
}

/// 上报响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportResponse {
    /// 是否成功
    pub success: bool,
    /// 消息
    pub message: Option<String>,
}

/// 上报管理器
pub struct ReportManager {
    /// 上报配置
    config: Arc<RwLock<Option<ReportConfig>>>,
    /// 统计管理器引用
    stats_manager: Arc<StatsManager>,
    /// 网络名称
    network_name: String,
    /// 节点名称
    node_name: String,
    /// 用户邮箱
    email: String,
    /// 上次上报的流量（用于计算增量）
    last_reported_traffic: Arc<RwLock<u64>>,
    /// HTTP客户端
    http_client: reqwest::Client,
    /// 后台任务
    background_task: ScopedTask<()>,
}

impl ReportManager {
    /// 创建新的上报管理器
    pub fn new(
        config: Option<ReportConfig>,
        stats_manager: Arc<StatsManager>,
        network_name: String,
        node_name: String,
        email: String,
    ) -> Arc<Self> {
        let config_arc = Arc::new(RwLock::new(config));
        let last_reported_traffic = Arc::new(RwLock::new(0u64));
        
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        // 创建后台任务的弱引用结构
        let config_weak = Arc::downgrade(&config_arc);
        let stats_manager_weak = Arc::downgrade(&stats_manager);
        let last_reported_traffic_weak = Arc::downgrade(&last_reported_traffic);
        let network_name_clone = network_name.clone();
        let node_name_clone = node_name.clone();
        let email_clone = email.clone();
        let http_client_clone = http_client.clone();

        let background_task = tokio::spawn(async move {
            Self::run_background_report_static(
                config_weak,
                stats_manager_weak,
                last_reported_traffic_weak,
                http_client_clone,
                network_name_clone,
                node_name_clone,
                email_clone,
            ).await;
        });

        Arc::new(Self {
            config: config_arc,
            stats_manager,
            network_name,
            node_name,
            email,
            last_reported_traffic,
            http_client,
            background_task: background_task.into(),
        })
    }

    /// 静态后台上报任务
    async fn run_background_report_static(
        config: std::sync::Weak<RwLock<Option<ReportConfig>>>,
        stats_manager: std::sync::Weak<StatsManager>,
        last_reported_traffic: std::sync::Weak<RwLock<u64>>,
        http_client: reqwest::Client,
        network_name: String,
        node_name: String,
        email: String,
    ) {
        loop {
            // 获取配置
            let Some(cfg_arc) = config.upgrade() else { break; };
            let cfg = cfg_arc.read().await;
            let Some(ref report_config) = *cfg else {
                drop(cfg);
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            };

            let heartbeat_interval = report_config.heartbeat_interval_minutes.max(1) as u64;
            let urls = report_config.report_urls.clone();
let token = report_config.report_token.clone();
            drop(cfg);

            // 等待心跳间隔
            tokio::time::sleep(Duration::from_secs(heartbeat_interval * 60)).await;

            // 收集数据并上报
            let Some(stats_mgr) = stats_manager.upgrade() else { break; };
            let Some(last_traffic_arc) = last_reported_traffic.upgrade() else { break; };

            Self::collect_and_report_static(
                &stats_mgr,
                &last_traffic_arc,
                &http_client,
                &network_name,
                &node_name,
                &email,
                &token,
                &urls,
                1, // 默认重置日期
            ).await;
        }
    }

    /// 静态方法：收集数据并上报
    async fn collect_and_report_static(
        stats_manager: &StatsManager,
        last_reported_traffic: &RwLock<u64>,
        http_client: &reqwest::Client,
        network_name: &str,
        node_name: &str,
        email: &str,
        token: &str,
        urls: &[String],
        reset_date: u32,
    ) {
        let label_set = LabelSet::new()
            .with_label("network_name", network_name.to_string());

        // 获取流量统计
        let tx_bytes = stats_manager
            .get_metric(MetricName::TrafficBytesTx, &label_set)
            .map(|m| m.value)
            .unwrap_or(0);

        let rx_bytes = stats_manager
            .get_metric(MetricName::TrafficBytesRx, &label_set)
            .map(|m| m.value)
            .unwrap_or(0);

        let total_bytes = tx_bytes + rx_bytes;

        // 计算流量增量
        let mut last_traffic = last_reported_traffic.write().await;
        let traffic_delta = if total_bytes > *last_traffic {
            total_bytes - *last_traffic
        } else {
            0
        };
        *last_traffic = total_bytes;
        drop(last_traffic);

        let reported_traffic_gb = traffic_delta as f64 / (1024.0 * 1024.0 * 1024.0);

// 获取连接数 (暂时使用固定值，后续可以扩展)
        let connection_count = 5; // TODO: 实现真实的连接数统计

        // 获取当前带宽（简化实现，使用最近的流量速率）
        let current_bandwidth = 0.0; // TODO: 实现带宽计算

        // 构建上报请求
        let report_request = ReportRequest {
            node_name: node_name.to_string(),
            email: email.to_string(),
            token: token.to_string(),
            current_bandwidth,
            reported_traffic: reported_traffic_gb,
            connection_count,
            reset_date,
            status: "online".to_string(),
        };

        // 向所有URL上报
        for url in urls {
            let full_url = if url.ends_with("/api/report") {
                url.clone()
            } else if url.ends_with('/') {
                format!("{}api/report", url)
            } else {
                format!("{}/api/report", url)
            };

            match Self::send_report_static(http_client, &full_url, &report_request).await {
                Ok(response) => {
                    if response.success {
                        tracing::info!(
                            "Successfully reported to {}: traffic={}GB, connections={}",
                            full_url,
                            reported_traffic_gb,
                            connection_count
                        );
                    } else {
                        tracing::warn!(
                            "Report to {} returned failure: {:?}",
                            full_url,
                            response.message
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to report to {}: {}", full_url, e);
                }
            }
        }
    }

    /// 静态方法：发送上报请求
    async fn send_report_static(
        http_client: &reqwest::Client,
        url: &str,
        request: &ReportRequest,
    ) -> Result<ReportResponse, Box<dyn std::error::Error + Send + Sync>> {
        let response = http_client
            .post(url)
            .json(request)
            .send()
            .await?;

        if response.status().is_success() {
            let report_response = response.json::<ReportResponse>().await?;
            Ok(report_response)
        } else {
            Ok(ReportResponse {
                success: false,
                message: Some(format!("HTTP error: {}", response.status())),
            })
        }
    }

    /// 更新配置
    pub async fn update_config(&self, new_config: Option<ReportConfig>) {
        let mut config = self.config.write().await;
        *config = new_config;
    }

    /// 手动触发上报
    pub async fn trigger_report(&self, reset_date: u32) {
        let cfg = self.config.read().await;
        let Some(ref report_config) = *cfg else {
            tracing::warn!("No report config available");
            return;
        };

        let urls = report_config.report_urls.clone();
let token = report_config.report_token.clone();
        drop(cfg);

        Self::collect_and_report_static(
            &self.stats_manager,
            &self.last_reported_traffic,
            &self.http_client,
            &self.network_name,
            &self.node_name,
            &self.email,
            &token,
            &urls,
            reset_date,
        ).await;
    }

    /// 获取上次上报的流量
    pub async fn get_last_reported_traffic(&self) -> u64 {
        *self.last_reported_traffic.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_report_manager_creation() {
        let stats_manager = Arc::new(StatsManager::new());
        let config = Some(ReportConfig {
            token: "test_token".to_string(),
            heartbeat_interval_minutes: 5,
            report_urls: vec!["http://example.com".to_string()],
        });

        let manager = ReportManager::new(
            config,
            stats_manager,
            "test_network".to_string(),
            "test_node".to_string(),
            "test@example.com".to_string(),
        );

        let last_traffic = manager.get_last_reported_traffic().await;
        assert_eq!(last_traffic, 0);
    }

    #[test]
    fn test_report_request_serialization() {
        let request = ReportRequest {
            node_name: "test_node".to_string(),
            email: "test@example.com".to_string(),
            token: "test_token".to_string(),
            current_bandwidth: 50.5,
            reported_traffic: 0.5,
            connection_count: 5,
            reset_date: 1,
            status: "online".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("test_node"));
        assert!(json.contains("test@example.com"));
    }
}
