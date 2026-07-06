-- Optional human note on a monitor: terse machine names (payment.reconcile.sweep) stop
-- explaining themselves once there are a dozen of them.
ALTER TABLE heartbeat_monitors ADD COLUMN description TEXT;
