CREATE INDEX IF NOT EXISTS hosted_chat_requests_expiry ON hosted_chat_requests(expires_at,id);
