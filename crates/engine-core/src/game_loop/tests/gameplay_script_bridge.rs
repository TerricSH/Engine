#[cfg(all(
    test,
    feature = "subsystem-physics",
    feature = "subsystem-gameplay",
    feature = "subsystem-scripting-csharp"
))]
mod gameplay_script_bridge_tests {
    include!("gameplay_script_bridge/input_context.rs");
    include!("gameplay_script_bridge/lifecycle_events.rs");
    include!("gameplay_script_bridge/physics_queries.rs");
    include!("gameplay_script_bridge/physics_mutations.rs");
    include!("gameplay_script_bridge/damage.rs");
    include!("gameplay_script_bridge/query_filters.rs");
}
