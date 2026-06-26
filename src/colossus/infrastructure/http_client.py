"""Shared HTTP client settings for Colossus-owned outbound requests."""

from __future__ import annotations

from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any

import httpx

ClientCert = str | tuple[str, str] | tuple[str, str, str]


@dataclass(frozen=True)
class HttpClientConfig:
    ca_bundle: Path | None = None
    client_cert: Path | None = None
    client_key: Path | None = None
    client_key_password: str | None = None
    proxy_url: str | None = None
    trust_env: bool = True

    def with_ca_bundle(self, ca_bundle: Path | None) -> HttpClientConfig:
        if ca_bundle is None:
            return self
        return replace(self, ca_bundle=ca_bundle)

    @property
    def cert(self) -> ClientCert | None:
        if self.client_cert is None:
            return None
        if self.client_key is None:
            return str(self.client_cert)
        if self.client_key_password is None:
            return (str(self.client_cert), str(self.client_key))
        return (str(self.client_cert), str(self.client_key), self.client_key_password)

    def async_client_kwargs(
        self,
        *,
        timeout: float,
        follow_redirects: bool = False,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> dict[str, Any]:
        kwargs: dict[str, Any] = {
            "timeout": timeout,
            "follow_redirects": follow_redirects,
            "verify": str(self.ca_bundle) if self.ca_bundle else True,
            "trust_env": self.trust_env,
        }
        cert = self.cert
        if cert is not None:
            kwargs["cert"] = cert
        if transport is not None:
            kwargs["transport"] = transport
        elif self.proxy_url:
            kwargs["proxy"] = self.proxy_url
        return kwargs
