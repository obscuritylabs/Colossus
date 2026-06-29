"""Shared HTTP client settings for Colossus-owned outbound requests."""

from __future__ import annotations

import ssl
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
        verify = self._ssl_context() if self.ca_bundle or self.client_cert else True
        kwargs: dict[str, Any] = {
            "timeout": timeout,
            "follow_redirects": follow_redirects,
            "verify": verify,
            "trust_env": self.trust_env,
        }
        if transport is not None:
            kwargs["transport"] = transport
        elif self.proxy_url:
            kwargs["proxy"] = self.proxy_url
        return kwargs

    def _ssl_context(self) -> ssl.SSLContext:
        if self.ca_bundle is None:
            context = httpx.create_ssl_context(verify=True, trust_env=self.trust_env)
        elif self.ca_bundle.is_dir():
            context = ssl.create_default_context(capath=str(self.ca_bundle))
        else:
            context = ssl.create_default_context(cafile=str(self.ca_bundle))

        if self.client_cert is not None:
            # httpx 0.28 returns early for verify=<CA path>, so cert=... is skipped.
            context.load_cert_chain(
                certfile=str(self.client_cert),
                keyfile=str(self.client_key) if self.client_key else None,
                password=self.client_key_password,
            )
        return context
