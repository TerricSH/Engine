use std::collections::BTreeMap;

use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, HashDigest};

/// The cooking state of an asset in the dependency graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CookState {
    /// The asset has not been cooked yet.
    Uncooked,
    /// The asset is currently being cooked.
    Cooking,
    /// The asset has been cooked successfully, along with its content hash.
    Cooked(HashDigest),
    /// Cooking the asset failed with the given error message.
    Failed(String),
}

/// A node in the dependency graph representing a single asset.
#[derive(Clone, Debug)]
pub struct DependencyNode {
    /// Assets this node depends on (directed edges).
    pub deps: Vec<AssetId>,
    /// Assets that depend on this node (reverse edges).
    pub rev_deps: Vec<AssetId>,
    /// Current cooking state.
    pub state: CookState,
}

/// A directed graph of asset dependencies used by the cook pipeline.
///
/// Tracks which assets depend on which, and their cooking state.
/// Supports topological validation (all deps must be cooked before an
/// asset can be consumed).
#[derive(Clone, Debug, Default)]
pub struct DependencyGraph {
    nodes: BTreeMap<AssetId, DependencyNode>,
}

impl DependencyGraph {
    /// Create an empty dependency graph.
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
        }
    }

    /// Register an asset in the graph (no dependencies).
    ///
    /// If the asset is already present this is a no-op.
    pub fn register(&mut self, asset: AssetId) {
        self.nodes.entry(asset).or_insert(DependencyNode {
            deps: Vec::new(),
            rev_deps: Vec::new(),
            state: CookState::Uncooked,
        });
    }

    /// Add a dependency edge: `asset` depends on `depends_on`.
    ///
    /// Both assets are automatically registered if not already present.
    pub fn add_dependency(&mut self, asset: AssetId, depends_on: AssetId) {
        self.register(asset.clone());
        self.register(depends_on.clone());

        if let Some(node) = self.nodes.get_mut(&asset) {
            if !node.deps.contains(&depends_on) {
                node.deps.push(depends_on.clone());
            }
        }

        if let Some(rev) = self.nodes.get_mut(&depends_on) {
            if !rev.rev_deps.contains(&asset) {
                rev.rev_deps.push(asset);
            }
        }
    }

    /// Return the direct dependencies of `asset`.
    pub fn get_dependencies(&self, asset: &AssetId) -> Vec<AssetId> {
        self.nodes
            .get(asset)
            .map(|n| n.deps.clone())
            .unwrap_or_default()
    }

    /// Return the reverse dependencies of `asset` (things that depend on it).
    pub fn get_reverse_dependencies(&self, asset: &AssetId) -> Vec<AssetId> {
        self.nodes
            .get(asset)
            .map(|n| n.rev_deps.clone())
            .unwrap_or_default()
    }

    /// Get the current cooking state of an asset.
    pub fn get_state(&self, asset: &AssetId) -> Option<CookState> {
        self.nodes.get(asset).map(|n| n.state.clone())
    }

    /// Mark an asset as cooking.
    pub fn mark_cooking(&mut self, asset: &AssetId) {
        if let Some(node) = self.nodes.get_mut(asset) {
            node.state = CookState::Cooking;
        }
    }

    /// Mark an asset as successfully cooked with its content hash.
    pub fn mark_cooked(&mut self, asset: &AssetId, hash: HashDigest) {
        if let Some(node) = self.nodes.get_mut(asset) {
            node.state = CookState::Cooked(hash);
        }
    }

    /// Mark an asset as failed with an error message.
    pub fn mark_failed(&mut self, asset: &AssetId, error: String) {
        if let Some(node) = self.nodes.get_mut(asset) {
            node.state = CookState::Failed(error);
        }
    }

    /// Return `true` if the asset exists in the graph.
    pub fn contains(&self, asset: &AssetId) -> bool {
        self.nodes.contains_key(asset)
    }

    /// Returns the number of registered assets.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate over all (AssetId, DependencyNode) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&AssetId, &DependencyNode)> {
        self.nodes.iter()
    }

    /// Validate the dependency graph and produce diagnostics.
    ///
    /// Checks:
    /// - All dependencies exist as nodes in the graph.
    /// - No circular dependencies (DFS-based cycle detection).
    /// - All dependencies are successfully cooked.
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diags = Vec::new();

        // Check that all referenced dependencies exist
        for (id, node) in &self.nodes {
            for dep in &node.deps {
                if !self.nodes.contains_key(dep) {
                    diags.push({
                        let mut d = Diagnostic::new(
                            "COOK_MISSING_DEP",
                            DiagnosticSeverity::Error,
                            "cook",
                            format!(
                                "asset {:?} depends on {:?} which is not registered",
                                id.id, dep.id
                            ),
                        );
                        d.asset = Some(id.clone());
                        d
                    });
                }
            }
        }

        // Check for circular dependencies via DFS
        let cycles = self.find_cycles();
        for cycle in cycles {
            let cycle_str: Vec<String> = cycle.iter().map(|a| a.id.clone()).collect();
            diags.push({
                let mut d = Diagnostic::new(
                    "COOK_CYCLE",
                    DiagnosticSeverity::Error,
                    "cook",
                    format!("circular dependency detected: {}", cycle_str.join(" → ")),
                );
                d.asset = Some(cycle[0].clone());
                d
            });
        }

        // Check that all deps are cooked
        for (id, node) in &self.nodes {
            for dep in &node.deps {
                if let Some(dep_node) = self.nodes.get(dep) {
                    match &dep_node.state {
                        CookState::Uncooked | CookState::Cooking => {
                            diags.push({
                                let mut d = Diagnostic::new(
                                    "COOK_DEP_NOT_READY",
                                    DiagnosticSeverity::Error,
                                    "cook",
                                    format!(
                                        "dependency {:?} for {:?} is not yet cooked",
                                        dep.id, id.id
                                    ),
                                );
                                d.asset = Some(id.clone());
                                d
                            });
                        }
                        CookState::Failed(msg) => {
                            diags.push({
                                let mut d = Diagnostic::new(
                                    "COOK_DEP_FAILED",
                                    DiagnosticSeverity::Error,
                                    "cook",
                                    format!(
                                        "dependency {:?} for {:?} failed: {msg}",
                                        dep.id, id.id
                                    ),
                                );
                                d.asset = Some(id.clone());
                                d
                            });
                        }
                        CookState::Cooked(_) => {} // OK
                    }
                }
            }
        }

        diags
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Find all elementary cycles in the dependency graph.
    /// Uses a simple DFS-based cycle detection.
    fn find_cycles(&self) -> Vec<Vec<AssetId>> {
        let mut cycles = Vec::new();
        let mut visited = BTreeMap::new(); // AssetId -> visit state

        for id in self.nodes.keys() {
            if !visited.contains_key(id) {
                let mut path = Vec::new();
                self.dfs_cycle(id, &mut visited, &mut path, &mut cycles);
            }
        }

        cycles
    }

    fn dfs_cycle(
        &self,
        current: &AssetId,
        visited: &mut BTreeMap<AssetId, u8>, // 0=unvisited, 1=in-progress, 2=done
        path: &mut Vec<AssetId>,
        cycles: &mut Vec<Vec<AssetId>>,
    ) {
        match visited.get(current).copied().unwrap_or(0) {
            1 => {
                // Found a back edge → extract the cycle
                if let Some(pos) = path.iter().position(|p| p == current) {
                    cycles.push(path[pos..].to_vec());
                }
                return;
            }
            2 => return,
            _ => {}
        }

        visited.insert(current.clone(), 1);
        path.push(current.clone());

        if let Some(node) = self.nodes.get(current) {
            for dep in &node.deps {
                self.dfs_cycle(dep, visited, path, cycles);
            }
        }

        path.pop();
        visited.insert(current.clone(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::HashDigest;

    fn id(name: &str) -> AssetId {
        AssetId::new(name)
    }

    fn hash(val: u8) -> HashDigest {
        let mut h = [0u8; 32];
        h[0] = val;
        h
    }

    #[test]
    fn register_asset() {
        let mut g = DependencyGraph::new();
        g.register(id("mesh-cube"));
        assert!(g.contains(&id("mesh-cube")));
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn add_dependency_creates_nodes() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("shader-standard"), id("shader-common"));
        assert!(g.contains(&id("shader-standard")));
        assert!(g.contains(&id("shader-common")));
    }

    #[test]
    fn get_dependencies() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("scene-A"), id("mesh-cube"));
        g.add_dependency(id("scene-A"), id("material-default"));
        let deps = g.get_dependencies(&id("scene-A"));
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&id("mesh-cube")));
        assert!(deps.contains(&id("material-default")));
    }

    #[test]
    fn get_reverse_dependencies() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("scene-A"), id("mesh-cube"));
        g.add_dependency(id("scene-B"), id("mesh-cube"));
        let rev = g.get_reverse_dependencies(&id("mesh-cube"));
        assert_eq!(rev.len(), 2);
        assert!(rev.contains(&id("scene-A")));
        assert!(rev.contains(&id("scene-B")));
    }

    #[test]
    fn mark_cooked_state() {
        let mut g = DependencyGraph::new();
        g.register(id("mesh-cube"));
        g.mark_cooked(&id("mesh-cube"), hash(42));
        assert_eq!(
            g.get_state(&id("mesh-cube")),
            Some(CookState::Cooked(hash(42)))
        );
    }

    #[test]
    fn mark_failed_state() {
        let mut g = DependencyGraph::new();
        g.register(id("bad-asset"));
        g.mark_failed(&id("bad-asset"), "parse error".into());
        assert_eq!(
            g.get_state(&id("bad-asset")),
            Some(CookState::Failed("parse error".into()))
        );
    }

    #[test]
    fn mark_cooking_state() {
        let mut g = DependencyGraph::new();
        g.register(id("asset"));
        g.mark_cooking(&id("asset"));
        assert_eq!(g.get_state(&id("asset")), Some(CookState::Cooking));
    }

    #[test]
    fn diagnostics_missing_dep() {
        let mut g = DependencyGraph::new();
        g.register(id("a"));
        // Manually add a dep to an unregistered asset
        if let Some(node) = g.nodes.get_mut(&id("a")) {
            node.deps.push(id("missing"));
        }
        let diags = g.to_diagnostics();
        assert!(diags.iter().any(|d| d.code == "COOK_MISSING_DEP"));
    }

    #[test]
    fn diagnostics_cycle_detected() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("a"), id("b"));
        g.add_dependency(id("b"), id("c"));
        g.add_dependency(id("c"), id("a")); // cycle
        let diags = g.to_diagnostics();
        assert!(diags.iter().any(|d| d.code == "COOK_CYCLE"));
    }

    #[test]
    fn diagnostics_dep_not_ready() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("a"), id("b"));
        // b is not cooked → diag emitted for a
        let diags = g.to_diagnostics();
        assert!(diags.iter().any(|d| d.code == "COOK_DEP_NOT_READY"));
    }

    #[test]
    fn diagnostics_dep_failed() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("a"), id("b"));
        g.mark_failed(&id("b"), "boom".into());
        let diags = g.to_diagnostics();
        assert!(diags.iter().any(|d| d.code == "COOK_DEP_FAILED"));
    }

    #[test]
    fn no_cycles_is_clean() {
        let mut g = DependencyGraph::new();
        g.add_dependency(id("a"), id("b"));
        g.add_dependency(id("b"), id("c"));
        g.mark_cooked(&id("c"), hash(1));
        g.mark_cooked(&id("b"), hash(2));
        g.mark_cooked(&id("a"), hash(3));
        let diags = g.to_diagnostics();
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }
}
