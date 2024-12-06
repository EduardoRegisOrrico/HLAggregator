#[derive(Debug, Clone)]
pub struct AggregatorConfig {
    pub testnet: bool,
    pub retry_attempts: u32,
    pub timeout_ms: u64,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            testnet: false,
            retry_attempts: 3,
            timeout_ms: 5000,
        }
    }
} 