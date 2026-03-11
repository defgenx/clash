use crate::application::effects::CliCommand;

/// Translate a high-level CliCommand into raw CLI args.
pub fn to_args(command: &CliCommand) -> Vec<String> {
    match command {
        CliCommand::CreateTeam { name, description } => {
            vec![
                "team".to_string(),
                "create".to_string(),
                "--name".to_string(),
                name.to_string(),
                "--description".to_string(),
                description.to_string(),
            ]
        }
    }
}

pub fn resume_session_args(session_id: &str) -> Vec<String> {
    vec!["--resume".to_string(), session_id.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_create_args() {
        let cmd = CliCommand::CreateTeam {
            name: "my-team".to_string(),
            description: "A test team".to_string(),
        };
        let args = to_args(&cmd);
        assert_eq!(args[0], "team");
        assert_eq!(args[1], "create");
        assert!(args.contains(&"my-team".to_string()));
    }

    #[test]
    fn test_resume_args() {
        let args = resume_session_args("sess-123");
        assert_eq!(args, vec!["--resume", "sess-123"]);
    }
}
