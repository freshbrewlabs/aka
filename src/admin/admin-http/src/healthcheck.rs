use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct HealthcheckRes {
    pub version: String,
    pub healthy: bool,
}
