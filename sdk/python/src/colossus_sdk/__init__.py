"""Security-focused client primitives for the Colossus public API."""

from .client import (
    AgentRuns,
    ColossusClient,
    assert_compatible_server_info,
    is_terminal_run_update,
)
from .credential import StaticBearerCredential
from .endpoint import (
    EndpointDescriptor,
    assert_pinned_leaf_certificate,
    certificate_sha256,
)
from .error import (
    ColossusFieldViolation,
    ColossusRetryAfter,
    ColossusRpcError,
    ErrorOutcomeCertainty,
    decode_colossus_rpc_error,
)
from .watch import (
    RunFeedItem,
    RunFeedProtocolError,
    RunWatchReconciliation,
    watch_run,
)

__all__ = [
    "AgentRuns",
    "ColossusClient",
    "ColossusFieldViolation",
    "ColossusRetryAfter",
    "ColossusRpcError",
    "EndpointDescriptor",
    "ErrorOutcomeCertainty",
    "RunFeedItem",
    "RunFeedProtocolError",
    "RunWatchReconciliation",
    "StaticBearerCredential",
    "assert_compatible_server_info",
    "assert_pinned_leaf_certificate",
    "certificate_sha256",
    "decode_colossus_rpc_error",
    "is_terminal_run_update",
    "watch_run",
]
