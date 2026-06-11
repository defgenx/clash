#[derive(Debug, Clone)]
pub enum TeamAction {
    Create {
        name: String,
        description: String,
    },
    Delete {
        name: String,
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
    /// Change a member's model on the *current* team (resolved by the
    /// reducer from the view context). Empty model = inherit.
    SetMemberModel {
        member: String,
        model: String,
    },
    Refresh,
}
