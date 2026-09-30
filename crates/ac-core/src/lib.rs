//! Agent Control core library.
//!
//! Exposes the event store, state machine, session manager,
//! adapter contract, control API server, account manager,
//! and project registry.

pub mod account_manager;
pub mod adapter;
pub mod agy_auth;
pub mod agy_launch;
pub mod config;
pub mod credentials;
pub mod event_store;
pub mod interaction_hub;
pub mod ipc;
pub mod policy_engine;
pub mod project_registry;
pub mod session;
pub mod settings;
pub mod types;
pub mod ws;
