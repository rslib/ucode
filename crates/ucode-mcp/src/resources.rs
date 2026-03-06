/// Namespaced resource identifier: `mcp.<server>.<resource_name>`
pub fn namespaced_resource(server: &str, resource_name: &str) -> String {
    format!("mcp.{server}.{resource_name}")
}

/// Namespaced prompt identifier: `mcp.<server>.<prompt_name>`
pub fn namespaced_prompt(server: &str, prompt_name: &str) -> String {
    format!("mcp.{server}.{prompt_name}")
}

/// Parse a namespaced resource/prompt identifier back to (server, name).
/// Returns None if the format doesn't match `mcp.<server>.<name>`.
pub fn parse_namespaced(namespaced: &str) -> Option<(&str, &str)> {
    let rest = namespaced.strip_prefix("mcp.")?;
    let dot = rest.find('.')?;
    if dot == 0 || dot == rest.len() - 1 {
        return None;
    }
    Some((&rest[..dot], &rest[dot + 1..]))
}

/// Registry for discovered resources and prompts across MCP servers.
pub struct McpResourceRegistry {
    resources: Vec<NamespacedResource>,
    prompts: Vec<NamespacedPrompt>,
}

/// A resource with its server origin.
#[derive(Debug, Clone)]
pub struct NamespacedResource {
    pub server: String,
    pub namespaced_name: String,
    pub def: crate::types::McpResourceDef,
}

/// A prompt with its server origin.
#[derive(Debug, Clone)]
pub struct NamespacedPrompt {
    pub server: String,
    pub namespaced_name: String,
    pub def: crate::types::McpPromptDef,
}

impl McpResourceRegistry {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            prompts: Vec::new(),
        }
    }

    /// Register resources from a server. Returns number of collisions detected.
    pub fn register_resources(
        &mut self,
        server: &str,
        defs: Vec<crate::types::McpResourceDef>,
    ) -> usize {
        let mut collisions = 0;
        for def in defs {
            let ns = namespaced_resource(server, &def.name);
            if self.resources.iter().any(|r| r.namespaced_name == ns) {
                collisions += 1;
            } else {
                self.resources.push(NamespacedResource {
                    server: server.to_owned(),
                    namespaced_name: ns,
                    def,
                });
            }
        }
        collisions
    }

    /// Register prompts from a server. Returns number of collisions detected.
    pub fn register_prompts(
        &mut self,
        server: &str,
        defs: Vec<crate::types::McpPromptDef>,
    ) -> usize {
        let mut collisions = 0;
        for def in defs {
            let ns = namespaced_prompt(server, &def.name);
            if self.prompts.iter().any(|p| p.namespaced_name == ns) {
                collisions += 1;
            } else {
                self.prompts.push(NamespacedPrompt {
                    server: server.to_owned(),
                    namespaced_name: ns,
                    def,
                });
            }
        }
        collisions
    }

    /// List all registered resources.
    pub fn list_resources(&self) -> &[NamespacedResource] {
        &self.resources
    }

    /// List all registered prompts.
    pub fn list_prompts(&self) -> &[NamespacedPrompt] {
        &self.prompts
    }

    /// Find a resource by namespaced name.
    pub fn find_resource(&self, namespaced_name: &str) -> Option<&NamespacedResource> {
        self.resources
            .iter()
            .find(|r| r.namespaced_name == namespaced_name)
    }

    /// Find a prompt by namespaced name.
    pub fn find_prompt(&self, namespaced_name: &str) -> Option<&NamespacedPrompt> {
        self.prompts
            .iter()
            .find(|p| p.namespaced_name == namespaced_name)
    }

    /// List resources from a specific server.
    pub fn resources_by_server(&self, server: &str) -> Vec<&NamespacedResource> {
        self.resources
            .iter()
            .filter(|r| r.server == server)
            .collect()
    }

    /// List prompts from a specific server.
    pub fn prompts_by_server(&self, server: &str) -> Vec<&NamespacedPrompt> {
        self.prompts.iter().filter(|p| p.server == server).collect()
    }
}

impl Default for McpResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{McpPromptDef, McpResourceDef};

    fn make_resource(name: &str) -> McpResourceDef {
        McpResourceDef {
            uri: format!("file:///{name}"),
            name: name.to_owned(),
            description: None,
            mime_type: None,
        }
    }

    fn make_prompt(name: &str) -> McpPromptDef {
        McpPromptDef {
            name: name.to_owned(),
            description: None,
            arguments: None,
        }
    }

    #[test]
    fn test_namespaced_resource() {
        assert_eq!(
            namespaced_resource("myserver", "data_csv"),
            "mcp.myserver.data_csv"
        );
    }

    #[test]
    fn test_namespaced_prompt() {
        assert_eq!(
            namespaced_prompt("myserver", "summarize"),
            "mcp.myserver.summarize"
        );
    }

    #[test]
    fn test_parse_namespaced_valid() {
        let ns = namespaced_resource("srv", "res");
        let (server, name) = parse_namespaced(&ns).unwrap();
        assert_eq!(server, "srv");
        assert_eq!(name, "res");

        // name with dots should still parse correctly (only first dot splits)
        let ns2 = namespaced_prompt("srv", "a.b.c");
        let (s2, n2) = parse_namespaced(&ns2).unwrap();
        assert_eq!(s2, "srv");
        assert_eq!(n2, "a.b.c");
    }

    #[test]
    fn test_parse_namespaced_invalid() {
        assert!(parse_namespaced("tools.foo.bar").is_none()); // wrong prefix
        assert!(parse_namespaced("mcp.").is_none()); // no name part
        assert!(parse_namespaced("mcp.server").is_none()); // no second dot
        assert!(parse_namespaced("").is_none());
    }

    #[test]
    fn test_registry_register_resources() {
        let mut reg = McpResourceRegistry::new();
        let defs = vec![make_resource("alpha"), make_resource("beta")];
        let collisions = reg.register_resources("srv1", defs);
        assert_eq!(collisions, 0);
        assert_eq!(reg.list_resources().len(), 2);
        assert_eq!(reg.list_resources()[0].namespaced_name, "mcp.srv1.alpha");
        assert_eq!(reg.list_resources()[1].namespaced_name, "mcp.srv1.beta");
    }

    #[test]
    fn test_registry_register_prompts() {
        let mut reg = McpResourceRegistry::new();
        let defs = vec![make_prompt("greet"), make_prompt("farewell")];
        let collisions = reg.register_prompts("srv1", defs);
        assert_eq!(collisions, 0);
        assert_eq!(reg.list_prompts().len(), 2);
        assert_eq!(reg.list_prompts()[0].namespaced_name, "mcp.srv1.greet");
    }

    #[test]
    fn test_registry_collision_detection() {
        let mut reg = McpResourceRegistry::new();
        reg.register_resources("srv1", vec![make_resource("foo")]);
        // Registering the same name from the same server is a collision.
        let collisions = reg.register_resources("srv1", vec![make_resource("foo")]);
        assert_eq!(collisions, 1);
        // The original entry is preserved; no duplicate added.
        assert_eq!(reg.list_resources().len(), 1);

        // Same name from a different server is NOT a collision.
        let collisions2 = reg.register_resources("srv2", vec![make_resource("foo")]);
        assert_eq!(collisions2, 0);
        assert_eq!(reg.list_resources().len(), 2);
    }

    #[test]
    fn test_registry_find_resource() {
        let mut reg = McpResourceRegistry::new();
        reg.register_resources("srv1", vec![make_resource("data")]);
        let found = reg.find_resource("mcp.srv1.data");
        assert!(found.is_some());
        assert_eq!(found.unwrap().def.name, "data");
        assert!(reg.find_resource("mcp.srv1.missing").is_none());
    }

    #[test]
    fn test_registry_find_prompt() {
        let mut reg = McpResourceRegistry::new();
        reg.register_prompts("srv1", vec![make_prompt("ask")]);
        let found = reg.find_prompt("mcp.srv1.ask");
        assert!(found.is_some());
        assert_eq!(found.unwrap().def.name, "ask");
        assert!(reg.find_prompt("mcp.srv1.nope").is_none());
    }

    #[test]
    fn test_registry_by_server() {
        let mut reg = McpResourceRegistry::new();
        reg.register_resources("srv1", vec![make_resource("a"), make_resource("b")]);
        reg.register_resources("srv2", vec![make_resource("c")]);
        reg.register_prompts("srv1", vec![make_prompt("p1")]);
        reg.register_prompts("srv2", vec![make_prompt("p2"), make_prompt("p3")]);

        assert_eq!(reg.resources_by_server("srv1").len(), 2);
        assert_eq!(reg.resources_by_server("srv2").len(), 1);
        assert_eq!(reg.resources_by_server("srv3").len(), 0);

        assert_eq!(reg.prompts_by_server("srv1").len(), 1);
        assert_eq!(reg.prompts_by_server("srv2").len(), 2);
    }
}
