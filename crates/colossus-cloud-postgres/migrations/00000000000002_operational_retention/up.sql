-- Bound operational retention scans; canonical history/audit remains operator-retained.
CREATE INDEX outbox_delivered_expiry ON delivery_outbox(delivered_at,outbox_id) WHERE delivered_at IS NOT NULL;
CREATE INDEX oidc_flow_expiry ON oidc_flows(expires_at,id) WHERE expires_at IS NOT NULL;
