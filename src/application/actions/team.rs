#[derive(Debug, Clone)]
pub enum TeamAction {
    Create {
        name: String,
        description: String,
    },
    Delete {
        name: String,
    },
    /// Rename a team (moves its on-disk config and tasks dirs).
    Rename {
        old: String,
        new: String,
    },
    /// Replace a team's description.
    SetDescription {
        name: String,
        description: String,
    },
    /// Add a member (agent) to a team. Empty `agent_type` defaults to
    /// `general-purpose`; empty `model` means "inherit".
    AddMember {
        team: String,
        name: String,
        agent_type: String,
        model: String,
    },
    /// Remove a member from a team by name.
    RemoveMember {
        team: String,
        member: String,
    },
    /// Rename a member on the *current* team (resolved by the reducer from the
    /// view context, like `SetMemberModel`).
    RenameMember {
        old: String,
        new: String,
    },
    /// Change a member's model on the *current* team (resolved by the
    /// reducer from the view context). Empty model = inherit.
    SetMemberModel {
        member: String,
        model: String,
    },
    /// Change a member's agent type on the current team (empty = general-purpose).
    SetMemberType {
        member: String,
        agent_type: String,
    },
    /// Replace a member's system prompt on the current team.
    SetMemberPrompt {
        member: String,
        prompt: String,
    },
    Refresh,
}
