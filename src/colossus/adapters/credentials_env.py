"""Environment-backed credential broker."""

import os

from colossus.domain.errors import ColossusError
from colossus.ports.credentials import CredentialMaterial


class EnvCredentialBroker:
    """Resolve credential handles of the form ``env:VARIABLE_NAME``."""

    def resolve(self, credential_ref: str) -> CredentialMaterial:
        if not credential_ref.startswith("env:"):
            raise ColossusError("Credential refs must use the env:VARIABLE_NAME form.")
        env_name = credential_ref.removeprefix("env:").strip()
        if not env_name:
            raise ColossusError("Credential refs must name an environment variable.")
        value = os.environ.get(env_name)
        if not value:
            raise ColossusError(f"Credential environment variable is not set: {env_name}")
        return CredentialMaterial(ref=credential_ref, value=value)
