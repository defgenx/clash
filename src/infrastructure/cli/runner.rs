use crate::domain::ports::{CliGateway, CliOutput};
use crate::infrastructure::error::Result;

/// Production CLI runner using tokio::process.
pub struct RealCliRunner {
    pub claude_bin: String,
}

impl RealCliRunner {
    pub fn with_bin(bin: String) -> Self {
        Self { claude_bin: bin }
    }
}

impl CliGateway for RealCliRunner {
    async fn run(&self, args: &[String]) -> Result<CliOutput> {
        let output = tokio::process::Command::new(&self.claude_bin)
            .args(args)
            .output()
            .await?;

        Ok(CliOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// Mock CLI runner for tests.
#[cfg(test)]
pub struct MockCliRunner {
    pub responses: std::collections::HashMap<String, CliOutput>,
}

#[cfg(test)]
impl MockCliRunner {
    pub fn new() -> Self {
        Self {
            responses: std::collections::HashMap::new(),
        }
    }

    pub fn add_response(&mut self, key: &str, output: CliOutput) {
        self.responses.insert(key.to_string(), output);
    }
}

#[cfg(test)]
impl CliGateway for MockCliRunner {
    async fn run(&self, args: &[String]) -> Result<CliOutput> {
        let key = args.join(" ");
        self.responses.get(&key).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No mock response for: {}", key),
            )
            .into()
        })
    }
}
