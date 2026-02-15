use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;
use tracing::{info, warn};

/// Bootstrap an Ubuntu 24.04 (Noble) rootfs using debootstrap.
///
/// The rootfs is created at `{storage_path}/{container_name}/`.
/// Post-bootstrap configuration:
/// - Empty machine-id (regenerated on first boot)
/// - systemd-networkd enabled
/// - systemd-resolved disabled (uses static resolv.conf)
pub async fn bootstrap_ubuntu(container_name: &str, storage_path: &Path) -> Result<()> {
    let rootfs = storage_path.join(container_name);

    info!(container = container_name, rootfs = %rootfs.display(), "Bootstrapping Ubuntu 24.04 rootfs");

    // Ensure parent directory exists
    tokio::fs::create_dir_all(storage_path).await
        .context("failed to create storage directory")?;

    // Run debootstrap
    let output = Command::new("debootstrap")
        .args([
            "--variant=minbase",
            "noble",
            &rootfs.to_string_lossy(),
            "http://archive.ubuntu.com/ubuntu",
        ])
        .output()
        .await
        .context("failed to run debootstrap")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("debootstrap failed: {stderr}");
    }

    info!(container = container_name, "debootstrap complete, configuring rootfs");

    // Post-bootstrap configuration

    // 1. Empty machine-id (will be regenerated on first boot)
    tokio::fs::write(rootfs.join("etc/machine-id"), "").await
        .context("failed to write machine-id")?;

    // 1b. Set hostname to the container name
    tokio::fs::write(rootfs.join("etc/hostname"), format!("{container_name}\n")).await
        .context("failed to write hostname")?;

    // 2. Enable systemd-networkd for DHCP
    let networkd_link = rootfs.join("etc/systemd/system/multi-user.target.wants/systemd-networkd.service");
    if let Some(parent) = networkd_link.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let _ = tokio::fs::symlink(
        "/lib/systemd/system/systemd-networkd.service",
        &networkd_link,
    ).await;

    // 3. Mask systemd-resolved to prevent DNS interference
    // (containers use static resolv.conf pointing to HomeRoute DNS)
    let resolved_mask = rootfs.join("etc/systemd/system/systemd-resolved.service");
    let _ = tokio::fs::symlink("/dev/null", &resolved_mask).await;

    // 4. Disable IPv6 (containers lack IPv6 routing, causes DNS failures in Node.js)
    let sysctl_dir = rootfs.join("etc/sysctl.d");
    tokio::fs::create_dir_all(&sysctl_dir).await.ok();
    tokio::fs::write(
        sysctl_dir.join("99-disable-ipv6.conf"),
        "net.ipv6.conf.all.disable_ipv6 = 1\nnet.ipv6.conf.default.disable_ipv6 = 1\n",
    ).await.context("failed to write sysctl ipv6 config")?;

    // 4b. Force curl to use IPv4 (sysctl alone doesn't prevent AAAA DNS queries)
    tokio::fs::write(rootfs.join("root/.curlrc"), "--ipv4\n").await
        .context("failed to write .curlrc")?;

    // 4c. Prefer IPv4 in getaddrinfo (affects all glibc-based DNS resolution)
    tokio::fs::write(
        rootfs.join("etc/gai.conf"),
        "precedence ::ffff:0:0/96  100\n",
    ).await.context("failed to write gai.conf")?;

    // 5. Install essential packages in the rootfs via chroot
    // (dbus is needed for machinectl shell, curl for runtime installs)
    let setup_script = r#"
        apt-get update -qq 2>/dev/null
        apt-get install -y -qq dbus systemd-sysv iproute2 curl ca-certificates e2fsprogs 2>/dev/null
        systemctl enable systemd-networkd 2>/dev/null || true
        systemctl mask systemd-resolved 2>/dev/null || true
        chattr +i /etc/resolv.conf 2>/dev/null || true
    "#;

    // Use chroot to install packages in the rootfs
    let chroot_output = Command::new("chroot")
        .arg(&rootfs)
        .args(["/bin/bash", "-c", setup_script])
        .output()
        .await;

    match chroot_output {
        Ok(o) if o.status.success() => {
            info!(container = container_name, "Essential packages installed in rootfs");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!(container = container_name, "chroot package install had issues: {stderr}");
        }
        Err(e) => {
            warn!(container = container_name, "chroot failed: {e}, container may need manual setup");
        }
    }

    info!(container = container_name, "Ubuntu 24.04 rootfs bootstrap complete");
    Ok(())
}
