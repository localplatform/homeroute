# Containers V2 — systemd-nspawn Migration Tracking

## Status: Implemented — Ready for Testing

## Overview

Migration from LXC/LXD to systemd-nspawn as a parallel container runtime. The two systems coexist — LXD containers are NOT impacted.

## Architecture

- **New crate**: `hr-container` — NspawnClient wrapper for machinectl/systemd-nspawn
- **Container naming**: `hr-v2-{slug}` (nspawn) vs `hr-{slug}` (LXD, unchanged)
- **Network**: Shared `br-lan` bridge (system bridge, not LXD-managed)
- **Guest interface**: `host0` (nspawn) vs `eth0` (LXD)
- **Agent**: Same `hr-agent` binary, same WebSocket auth — only `interface` config differs
- **Storage**: Configurable per host (`container_storage_path`)

## Phases

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | `hr-container` crate (NspawnClient + rootfs bootstrap) | Done |
| 2 | AgentRegistry `create_application_headless()` | Done |
| 3 | ContainerManager + API routes `/api/containers/*` | Done |
| 4 | main.rs integration | Done |
| 5 | Frontend — ContainersV2 page | Done |
| 6 | Configurable storage path per host | Done |
| 7 | hr-host-agent nspawn support | Done |
| 8 | LXD → nspawn migration | Done |
| 9 | Inter-host nspawn migration (streaming tar pipe) | Done |

## Dependencies

### System packages required
```bash
# Local host
apt install systemd-container debootstrap

# Remote hosts (installed automatically by deploy_host_agent)
ssh {host} "apt-get install -y systemd-container debootstrap"
```

## Files Created

- `crates/hr-container/` — New crate
- `crates/hr-api/src/container_manager.rs` — ContainerManager
- `crates/hr-api/src/routes/containers.rs` — API routes
- `web/src/pages/ContainersV2.jsx` — Frontend page

## Files Modified

- `crates/Cargo.toml` — workspace members
- `crates/hr-api/Cargo.toml` — dependency
- `crates/hr-api/src/state.rs` — ApiState field
- `crates/hr-api/src/routes/mod.rs` — module declaration
- `crates/hr-api/src/lib.rs` — route mounting
- `crates/hr-registry/src/state.rs` — headless method
- `crates/hr-registry/src/protocol.rs` — nspawn message variants
- `crates/hr-host-agent/Cargo.toml` — dependency
- `crates/hr-host-agent/src/main.rs` — nspawn handlers
- `crates/homeroute/Cargo.toml` — dependency
- `crates/homeroute/src/main.rs` — ContainerManager init
- `web/src/App.jsx` — route
- `web/src/components/Sidebar.jsx` — nav item
- `web/src/api/client.js` — API functions
- `web/src/pages/Hosts.jsx` — storage path field

## Notes

- Full plan: `/root/.claude/plans/sunny-splashing-key.md`
- Zero impact on existing LXD containers guaranteed
- `crates/hr-lxd/` is NOT modified
