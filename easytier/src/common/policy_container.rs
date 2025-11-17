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
