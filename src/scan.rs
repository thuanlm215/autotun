//! Pure remote-scan reconciliation (no SSH side effects).
//!
//! [`plan_scan`] updates presence / labels / missing counters on existing local
//! tunnels and returns the enable / cancel / discover actions the UI layer
//! should apply through the SSH control socket.

use crate::ports::{Direction, RemoteListener, Tunnel};

/// Consecutive successful scans a remote listener must be absent from before
/// its local tunnel is considered down and any active forward is cancelled.
pub const MISSING_THRESHOLD: u8 = 2;

/// Side-effect free description of what the next SSH operations should be.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanAction {
    /// Auto-enable an existing local tunnel at this index.
    Enable(usize),
    /// Cancel the active forward for the tunnel at this index.
    Cancel {
        index: usize,
        bind: u16,
        source: u16,
    },
    /// Append a newly discovered remote listener (optionally auto-enable it).
    Discover { tunnel: Tunnel, enable: bool },
}

/// Update local tunnel presence from a remote listener snapshot and return
/// the SSH actions that should follow.
///
/// Rules:
/// - Discovered local tunnels that reappear are marked present; if
///   `auto_forward` is on and the tunnel is not `manual_off`, it is queued
///   for enable.
/// - A listener must be missing for [`MISSING_THRESHOLD`] consecutive scans
///   before `present` flips false and an active forward is cancelled.
///   Cancellation does **not** set `manual_off` — so when the service returns
///   and auto-forward is on, the tunnel is re-enabled.
/// - Manual-off tunnels (`manual_off == true`) are never auto-enabled, even
///   after the remote service restarts.
/// - New remote ports become new discovered tunnels (enabled when auto-forward
///   is on). Reverse tunnels are ignored by discovery.
pub fn plan_scan(
    tunnels: &mut [Tunnel],
    found: &[RemoteListener],
    auto_forward: bool,
) -> Vec<ScanAction> {
    let mut actions = Vec::new();

    for (index, tunnel) in tunnels.iter_mut().enumerate() {
        if tunnel.direction != Direction::Local {
            continue;
        }
        if let Some(listener) = found.iter().find(|l| l.port == tunnel.source_port) {
            tunnel.present = true;
            tunnel.missing_scans = 0;
            apply_auto_label(tunnel, listener.process.as_deref());
            if auto_forward && !tunnel.enabled && !tunnel.manual_off {
                actions.push(ScanAction::Enable(index));
            }
        } else {
            tunnel.missing_scans = tunnel.missing_scans.saturating_add(1);
            if tunnel.missing_scans >= MISSING_THRESHOLD {
                tunnel.present = false;
                if tunnel.enabled {
                    let bind = tunnel.bind_port.expect("enabled tunnel has bind port");
                    actions.push(ScanAction::Cancel {
                        index,
                        bind,
                        source: tunnel.source_port,
                    });
                }
            }
        }
    }

    for listener in found {
        if tunnels
            .iter()
            .any(|t| t.direction == Direction::Local && t.source_port == listener.port)
        {
            continue;
        }
        let tunnel = tunnel_from_listener(listener.clone());
        actions.push(ScanAction::Discover {
            enable: auto_forward,
            tunnel,
        });
    }

    actions
}

pub fn tunnel_from_listener(listener: RemoteListener) -> Tunnel {
    let mut tunnel = Tunnel::local(listener.port);
    if let Some(process) = listener.process {
        tunnel.label = process;
    }
    tunnel
}

pub fn apply_auto_label(tunnel: &mut Tunnel, process: Option<&str>) {
    if tunnel.label.is_empty()
        && let Some(process) = process
        && !process.is_empty()
    {
        tunnel.label = process.to_owned();
    }
}

/// In-memory application of [`plan_scan`]: enable/cancel succeed immediately
/// without SSH (`bind_port` falls back to `requested_port` on enable).
///
/// Used by unit tests and the opt-in SSH integration suite.
pub fn apply_scan_in_memory(
    tunnels: &mut Vec<Tunnel>,
    found: &[RemoteListener],
    auto_forward: bool,
) {
    let actions = plan_scan(tunnels, found, auto_forward);
    for action in actions {
        match action {
            ScanAction::Enable(index) => {
                let tunnel = &mut tunnels[index];
                tunnel.enabled = true;
                tunnel.bind_port = Some(tunnel.requested_port);
                tunnel.error = None;
            }
            ScanAction::Cancel { index, .. } => {
                let tunnel = &mut tunnels[index];
                tunnel.enabled = false;
                // bind_port kept so tests can inspect last mapping if needed
            }
            ScanAction::Discover { mut tunnel, enable } => {
                if enable {
                    tunnel.enabled = true;
                    tunnel.bind_port = Some(tunnel.requested_port);
                }
                tunnels.push(tunnel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::RemoteListener;

    fn listener(port: u16) -> RemoteListener {
        RemoteListener {
            port,
            process: None,
        }
    }

    fn listener_named(port: u16, name: &str) -> RemoteListener {
        RemoteListener {
            port,
            process: Some(name.into()),
        }
    }

    #[test]
    fn discovers_new_ports_and_auto_enables() {
        let mut tunnels = Vec::new();
        apply_scan_in_memory(&mut tunnels, &[listener(3000), listener(3001)], true);
        assert_eq!(tunnels.len(), 2);
        assert!(tunnels.iter().all(|t| t.enabled && t.discovered && t.present));
        assert_eq!(tunnels[0].source_port, 3000);
        assert_eq!(tunnels[1].source_port, 3001);
    }

    #[test]
    fn discovers_without_auto_forward_leaves_tunnels_off() {
        let mut tunnels = Vec::new();
        apply_scan_in_memory(&mut tunnels, &[listener(4000)], false);
        assert_eq!(tunnels.len(), 1);
        assert!(!tunnels[0].enabled);
        assert!(tunnels[0].present);
    }

    #[test]
    fn manual_off_blocks_auto_enable_while_service_stays_up() {
        let mut tunnel = Tunnel::local(5000);
        tunnel.enabled = false;
        tunnel.manual_off = true;
        tunnel.bind_port = None;
        let mut tunnels = vec![tunnel];

        apply_scan_in_memory(&mut tunnels, &[listener(5000)], true);
        assert!(!tunnels[0].enabled);
        assert!(tunnels[0].manual_off);
        assert!(tunnels[0].present);
    }

    #[test]
    fn manual_off_still_blocks_auto_enable_after_service_restart() {
        // User disabled while service was up.
        let mut tunnel = Tunnel::local(5100);
        tunnel.manual_off = true;
        tunnel.enabled = false;
        let mut tunnels = vec![tunnel];

        // Service disappears for two scans → TARGET DOWN.
        apply_scan_in_memory(&mut tunnels, &[], true);
        apply_scan_in_memory(&mut tunnels, &[], true);
        assert!(!tunnels[0].present);
        assert!(tunnels[0].manual_off);

        // Service restarts — must NOT auto-forward because manual_off sticks.
        apply_scan_in_memory(&mut tunnels, &[listener(5100)], true);
        assert!(tunnels[0].present);
        assert!(!tunnels[0].enabled);
        assert!(tunnels[0].manual_off);
    }

    #[test]
    fn auto_disabled_when_service_dies_is_reenabled_on_restart() {
        // Auto-forward path: tunnel was ON, user did not manual-off.
        let mut tunnel = Tunnel::local(5200);
        tunnel.enabled = true;
        tunnel.bind_port = Some(5200);
        tunnel.manual_off = false;
        let mut tunnels = vec![tunnel];

        // One missing scan is not enough.
        apply_scan_in_memory(&mut tunnels, &[], true);
        assert!(tunnels[0].enabled);
        assert!(tunnels[0].present);
        assert_eq!(tunnels[0].missing_scans, 1);

        // Second missing scan cancels the forward.
        apply_scan_in_memory(&mut tunnels, &[], true);
        assert!(!tunnels[0].enabled);
        assert!(!tunnels[0].present);
        assert!(!tunnels[0].manual_off);

        // Service returns → auto-forward again.
        apply_scan_in_memory(&mut tunnels, &[listener(5200)], true);
        assert!(tunnels[0].enabled);
        assert!(tunnels[0].present);
        assert!(!tunnels[0].manual_off);
    }

    #[test]
    fn single_missed_scan_does_not_mark_target_down() {
        let mut tunnel = Tunnel::local(5300);
        tunnel.enabled = true;
        tunnel.bind_port = Some(5300);
        let mut tunnels = vec![tunnel];

        apply_scan_in_memory(&mut tunnels, &[], true);
        assert!(tunnels[0].present);
        assert!(tunnels[0].enabled);
        assert_eq!(tunnels[0].missing_scans, 1);
    }

    #[test]
    fn reappearance_resets_missing_counter() {
        let mut tunnel = Tunnel::local(5400);
        tunnel.enabled = true;
        tunnel.bind_port = Some(5400);
        let mut tunnels = vec![tunnel];

        apply_scan_in_memory(&mut tunnels, &[], true);
        assert_eq!(tunnels[0].missing_scans, 1);
        apply_scan_in_memory(&mut tunnels, &[listener(5400)], true);
        assert_eq!(tunnels[0].missing_scans, 0);
        assert!(tunnels[0].enabled);
        assert!(tunnels[0].present);
    }

    #[test]
    fn fills_empty_label_from_process_name_only() {
        let mut tunnel = Tunnel::local(5500);
        tunnel.label = String::new();
        let mut tunnels = vec![tunnel];
        apply_scan_in_memory(&mut tunnels, &[listener_named(5500, "node")], false);
        assert_eq!(tunnels[0].label, "node");

        tunnels[0].label = "custom".into();
        apply_scan_in_memory(&mut tunnels, &[listener_named(5500, "other")], false);
        assert_eq!(tunnels[0].label, "custom");
    }

    #[test]
    fn does_not_duplicate_existing_ports() {
        let mut tunnels = vec![Tunnel::local(5600)];
        apply_scan_in_memory(&mut tunnels, &[listener(5600), listener(5600)], true);
        assert_eq!(tunnels.len(), 1);
    }

    #[test]
    fn reverse_tunnels_are_not_touched_by_discovery() {
        let mut reverse = Tunnel::reverse(8080);
        reverse.enabled = true;
        reverse.bind_port = Some(8080);
        let mut tunnels = vec![reverse];
        apply_scan_in_memory(&mut tunnels, &[listener(8080)], true);
        // Reverse row stays; a separate local discover may be added for 8080.
        assert_eq!(tunnels[0].direction, Direction::Reverse);
        assert!(tunnels[0].enabled);
        assert_eq!(tunnels.len(), 2);
        assert_eq!(tunnels[1].direction, Direction::Local);
        assert_eq!(tunnels[1].source_port, 8080);
    }

    #[test]
    fn plan_scan_emits_cancel_only_after_threshold() {
        let mut tunnel = Tunnel::local(5700);
        tunnel.enabled = true;
        tunnel.bind_port = Some(5700);
        let mut tunnels = vec![tunnel];

        let actions = plan_scan(&mut tunnels, &[], true);
        assert!(actions.is_empty());
        assert_eq!(tunnels[0].missing_scans, 1);

        let actions = plan_scan(&mut tunnels, &[], true);
        assert_eq!(
            actions,
            vec![ScanAction::Cancel {
                index: 0,
                bind: 5700,
                source: 5700,
            }]
        );
    }

    #[test]
    fn plan_scan_enable_skipped_when_manual_off() {
        let mut tunnel = Tunnel::local(5800);
        tunnel.manual_off = true;
        let mut tunnels = vec![tunnel];
        let actions = plan_scan(&mut tunnels, &[listener(5800)], true);
        assert!(actions.is_empty());
    }
}
