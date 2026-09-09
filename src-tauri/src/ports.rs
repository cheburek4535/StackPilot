//! Shared, canonical port model for generated projects and launch profiles.
//!
//! Both `project_creator` (scaffolds, Dockerfiles, docker-compose, env
//! examples, LOCAL_INFRA.md, README) and `devlauncher` (profile builder,
//! analyzer) used to hardcode ports independently, which produced projects
//! whose components collided on the same host port (e.g. a NestJS backend
//! and a local Grafana both on 3000, or an Airflow container and a
//! Spring Boot app both on 8080).
//!
//! This module is the single source of truth:
//! - [`framework_default_port`] — the canonical port a framework scaffold binds;
//! - [`tool_host_ports`] / [`tool_container_ports`] — compose port mapping per tool;
//! - [`PortAllocator`] — deterministic conflict resolution (bump to next free port);
//! - [`PortPlan`] — one allocation pass over a whole wizard stack, shared by
//!   project_creator's generated artifacts and devlauncher's launch profiles.

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Deterministic port allocation
// ---------------------------------------------------------------------------

/// Deterministic port allocator: claims exact ports and allocates the first
/// free port at or above a preferred value on collision.
///
/// The same stack always produces the same allocation (no OS queries, no
/// randomness), so generated artifacts and launch profiles agree even when
/// they are computed in different modules or at different times.
#[derive(Debug, Clone, Default)]
pub struct PortAllocator {
    taken: HashSet<u16>,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a port as taken without reallocating it. Used to pre-claim
    /// ports that are already owned by something else (e.g. published by
    /// an existing docker-compose file).
    pub fn claim(&mut self, port: u16) {
        if port != 0 {
            self.taken.insert(port);
        }
    }

    /// Allocate a port: `preferred` when free, otherwise the first free
    /// port above it. Port 0 is never returned.
    pub fn reserve(&mut self, preferred: u16) -> u16 {
        let mut p = preferred.max(1);
        while self.taken.contains(&p) {
            p = p.wrapping_add(1);
            if p == 0 {
                p = 1;
            }
        }
        self.taken.insert(p);
        p
    }

    pub fn is_taken(&self, port: u16) -> bool {
        self.taken.contains(&port)
    }

    /// All currently claimed/allocated ports, sorted.
    pub fn taken_ports(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self.taken.iter().copied().collect();
        v.sort_unstable();
        v
    }
}

// ---------------------------------------------------------------------------
// Canonical framework ports
// ---------------------------------------------------------------------------

/// Canonical default port for a framework id (wizard ids and devlauncher
/// ids). These match what the generated scaffold actually binds: the wizard
/// scaffolds FastAPI on 8000, Flask on 5000, Gin on 8080, NestJS/Express on
/// 3000, Spring Boot on 8080, etc.
pub fn framework_default_port(fw: &str) -> Option<u16> {
    Some(match fw {
        // Node backends
        "express" | "fastify" | "hono" | "nest" | "nestjs" => 3000,
        // Node full-stack / frontends
        "nextjs" | "next" | "nuxt" | "nuxtjs" | "sveltekit" => 3000,
        "vite" | "vite-react" | "vite-vue" | "vite-svelte" | "react" | "vue" | "svelte"
        | "solid" | "solidjs" | "plasmo" => 5173,
        "angular" => 4200,
        "expo" | "react-native" => 8081,
        // Python
        "fastapi" | "litestar" => 8000,
        "flask" => 5000,
        "django" => 8000,
        // Go (the wizard's Gin scaffold binds :8080)
        "gin" | "echo" | "fiber" | "chi" | "gorilla" => 8080,
        // Rust
        "axum" | "actix" | "actix-web" | "rocket" | "warp" => 3000,
        // JVM
        "spring-boot" | "spring" => 8080,
        "ktor" => 3000,
        // .NET
        "aspnetcore" | "aspnet" | "blazor" => 8080,
        // PHP
        "laravel" | "symfony" => 8000,
        // Elixir / Swift
        "phoenix" => 4000,
        "vapor" => 8080,
        // Ruby
        "rails" => 3000,
        // Desktop shell
        "tauri" => 1420,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Canonical compose tool ports
// ---------------------------------------------------------------------------

/// Canonical compose service name for a wizard tool id. `None` for tools
/// that are never deployed as compose services by the wizard. Service names
/// themselves are also recognized (so callers can filter a parsed compose
/// file by known infrastructure services).
pub fn tool_service_name(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "postgresql" | "postgres" => "postgres",
        "redis" => "redis",
        "mongodb" | "mongo" => "mongo",
        "mysql" => "mysql",
        "kafka" => "kafka",
        "clickhouse" => "clickhouse",
        "mailpit" => "mailpit",
        "opentelemetry" | "otel-collector" => "otel-collector",
        "airflow" => "airflow",
        "grafana" => "grafana",
        _ => return None,
    })
}

/// Preferred HOST ports published by the generated compose for a tool.
/// Grafana's preferred host port is 3001 (its container port 3000 would
/// collide with the app service's default 3000); the allocator bumps any
/// preferred port that is already taken.
pub fn tool_host_ports(tool: &str) -> &'static [u16] {
    match tool {
        "postgresql" => &[5432],
        "redis" => &[6379],
        "mongodb" => &[27017],
        "mysql" => &[3306],
        "kafka" => &[9092],
        "clickhouse" => &[8123, 9000],
        "mailpit" => &[1025, 8025],
        "opentelemetry" => &[4317, 4318],
        "airflow" => &[8080],
        "grafana" => &[3001],
        _ => &[],
    }
}

/// Container ports inside the compose network for a tool. Host and container
/// ports coincide for every tool except Grafana (container 3000, host 3001+).
pub fn tool_container_ports(tool: &str) -> &'static [u16] {
    match tool {
        "grafana" => &[3000],
        other => tool_host_ports(other),
    }
}

// ---------------------------------------------------------------------------
// Stack-level port plan
// ---------------------------------------------------------------------------

/// The result of one allocation pass over a wizard stack: the host port of
/// the compose `app` service plus every host port the generated compose
/// publishes for the selected (docker-mode) tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPlan {
    /// Host port published by the generated compose `app` service. The app
    /// keeps its canonical framework port; only infra tools are remapped on
    /// collision (e.g. Airflow 8080 -> 8081 next to a Spring Boot app).
    pub app_port: u16,
    /// Allocated host ports per selected docker-mode tool, in tool order.
    pub tool_host_ports: Vec<(String, Vec<u16>)>,
}

impl PortPlan {
    /// Compute the plan for a wizard stack. Deterministic: identical inputs
    /// always produce identical ports.
    pub fn new(
        frameworks: &[String],
        tools: &[String],
        local_infra_tools: &[String],
    ) -> Self {
        let mut alloc = PortAllocator::new();
        // The app claims its canonical port FIRST; infra tools are remapped
        // around it, so the app's identity port never changes.
        let preferred = frameworks
            .first()
            .and_then(|f| framework_default_port(f))
            .unwrap_or(3000);
        let app_port = alloc.reserve(preferred);
        let mut allocated_map: Vec<(String, Vec<u16>)> = Vec::new();
        for tool in tools {
            // Locally installed tools are excluded from the compose file and
            // publish no container ports.
            if local_infra_tools.iter().any(|t| t == tool) {
                continue;
            }
            let preferred_ports = tool_host_ports(tool);
            if preferred_ports.is_empty() {
                continue;
            }
            let allocated: Vec<u16> =
                preferred_ports.iter().map(|p| alloc.reserve(*p)).collect();
            allocated_map.push((tool.clone(), allocated));
        }
        Self {
            app_port,
            tool_host_ports: allocated_map,
        }
    }

    /// Allocated host ports for a tool (empty when the tool is absent or
    /// runs locally).
    pub fn tool_ports(&self, tool: &str) -> &[u16] {
        self.tool_host_ports
            .iter()
            .find(|(t, _)| t == tool)
            .map(|(_, p)| p.as_slice())
            .unwrap_or(&[])
    }

    /// Every host port the generated compose publishes (app service + tools).
    pub fn compose_host_ports(&self) -> Vec<u16> {
        let mut out = vec![self.app_port];
        for (_, ports) in &self.tool_host_ports {
            out.extend(ports.iter().copied());
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Host port for a LOCAL (non-docker) Grafana install. Grafana's native
    /// port 3000 collides with apps that listen on 3000, so it moves to 3001
    /// in that case (matching the docker-mode remap).
    pub fn grafana_local_port(&self) -> u16 {
        if self.app_port == 3000 {
            3001
        } else {
            3000
        }
    }
}

/// Pick the first free local dev-server port: `preferred`, bumped past every
/// port in `reserved`. Used by devlauncher to keep local dev servers (and
/// their waits/URLs) off ports already claimed by compose or other services.
pub fn local_dev_port(preferred: u16, reserved: &[u16]) -> u16 {
    let mut alloc = PortAllocator::new();
    for p in reserved {
        alloc.claim(*p);
    }
    alloc.reserve(preferred)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(frameworks: &[&str], tools: &[&str], local: &[&str]) -> (Vec<String>, Vec<String>, Vec<String>) {
        (
            frameworks.iter().map(|s| s.to_string()).collect(),
            tools.iter().map(|s| s.to_string()).collect(),
            local.iter().map(|s| s.to_string()).collect(),
        )
    }

    #[test]
    fn allocator_bumps_collisions_in_order() {
        let mut a = PortAllocator::new();
        assert_eq!(a.reserve(3000), 3000);
        assert_eq!(a.reserve(3000), 3001);
        assert_eq!(a.reserve(3000), 3002);
        assert_eq!(a.reserve(8080), 8080);
        assert_eq!(a.reserve(3000), 3003);
        assert!(a.is_taken(3000));
        assert_eq!(a.taken_ports(), vec![3000, 3001, 3002, 3003, 8080]);
    }

    #[test]
    fn claim_blocks_reserve() {
        let mut a = PortAllocator::new();
        a.claim(5173);
        assert_eq!(a.reserve(5173), 5174);
    }

    #[test]
    fn framework_default_ports_cover_wizard_frameworks() {
        assert_eq!(framework_default_port("nest"), Some(3000));
        assert_eq!(framework_default_port("express"), Some(3000));
        assert_eq!(framework_default_port("fastapi"), Some(8000));
        assert_eq!(framework_default_port("flask"), Some(5000));
        assert_eq!(framework_default_port("django"), Some(8000));
        assert_eq!(framework_default_port("gin"), Some(8080));
        assert_eq!(framework_default_port("spring-boot"), Some(8080));
        assert_eq!(framework_default_port("vapor"), Some(8080));
        assert_eq!(framework_default_port("phoenix"), Some(4000));
        assert_eq!(framework_default_port("nextjs"), Some(3000));
        assert_eq!(framework_default_port("react"), Some(5173));
        assert_eq!(framework_default_port("unknown-fw"), None);
    }

    #[test]
    fn plan_keeps_app_port_and_remaps_airflow() {
        let (fw, tools, local) = stack(&["spring-boot"], &["airflow", "postgresql"], &[]);
        let plan = PortPlan::new(&fw, &tools, &local);
        assert_eq!(plan.app_port, 8080);
        assert_eq!(plan.tool_ports("airflow"), &[8081]);
        assert_eq!(plan.tool_ports("postgresql"), &[5432]);
    }

    #[test]
    fn plan_grafana_gets_3001_and_survives_app_3000() {
        let (fw, tools, local) = stack(&["nest"], &["grafana"], &[]);
        let plan = PortPlan::new(&fw, &tools, &local);
        assert_eq!(plan.app_port, 3000);
        assert_eq!(plan.tool_ports("grafana"), &[3001]);
        assert_eq!(plan.compose_host_ports(), vec![3000, 3001]);
        // Local (non-docker) Grafana must also move off 3000 next to a 3000 app.
        assert_eq!(plan.grafana_local_port(), 3001);
    }

    #[test]
    fn plan_grafana_stays_on_3000_for_non_3000_apps() {
        let (fw, tools, local) = stack(&["django"], &["grafana"], &[]);
        let plan = PortPlan::new(&fw, &tools, &local);
        assert_eq!(plan.app_port, 8000);
        assert_eq!(plan.tool_ports("grafana"), &[3001]);
        assert_eq!(plan.grafana_local_port(), 3000);
    }

    #[test]
    fn plan_excludes_local_infra_tools() {
        let (fw, tools, local) = stack(&["nest"], &["grafana", "postgresql"], &["grafana"]);
        let plan = PortPlan::new(&fw, &tools, &local);
        assert_eq!(plan.tool_ports("grafana"), &[] as &[u16]);
        assert_eq!(plan.tool_ports("postgresql"), &[5432]);
    }

    #[test]
    fn plan_is_deterministic() {
        let (fw1, tools1, local1) = stack(&["spring-boot"], &["airflow", "grafana", "kafka"], &[]);
        let (fw2, tools2, local2) = stack(&["spring-boot"], &["airflow", "grafana", "kafka"], &[]);
        assert_eq!(
            PortPlan::new(&fw1, &tools1, &local1),
            PortPlan::new(&fw2, &tools2, &local2)
        );
    }

    #[test]
    fn local_dev_port_bumps_off_reserved() {
        assert_eq!(local_dev_port(3000, &[3000, 3001, 3002]), 3003);
        assert_eq!(local_dev_port(5173, &[3000]), 5173);
        assert_eq!(local_dev_port(3001, &[3000, 3001]), 3002);
    }

    #[test]
    fn tool_ports_agree_with_compose_mappings() {
        assert_eq!(tool_container_ports("grafana"), &[3000]);
        assert_eq!(tool_host_ports("grafana"), &[3001]);
        assert_eq!(tool_container_ports("clickhouse"), &[8123, 9000]);
        assert_eq!(tool_host_ports("kafka"), &[9092]);
        assert_eq!(tool_service_name("postgresql"), Some("postgres"));
        assert_eq!(tool_service_name("opentelemetry"), Some("otel-collector"));
        assert_eq!(tool_service_name("docker"), None);
    }
}