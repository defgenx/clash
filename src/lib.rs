#![allow(dead_code, unused_imports)]

// Clean Architecture layers (dependency direction: inward only)
//
//   Infrastructure → Adapters → Application → Domain
//
// Domain:         Pure entities and port traits (no deps on outer layers)
// Application:    State, actions, effects, reducer (depends on domain)
// Adapters:       Input handling, rendering, views (translates app ↔ infra)
// Infrastructure: Filesystem, CLI, TUI, config (implements ports)

pub mod domain;
pub mod application;
pub mod adapters;
pub mod infrastructure;
