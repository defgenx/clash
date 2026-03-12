use crate::domain::error::Result;
use crate::domain::ports::{CliGateway, CliOutput};

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
