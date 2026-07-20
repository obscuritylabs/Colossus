"""In-memory credentials with redacted representations."""

from __future__ import annotations

from typing import NoReturn


class StaticBearerCredential:
    """A caller-supplied bearer token that never loads itself from ambient state."""

    __slots__ = ("__token",)

    def __init__(self, token: str) -> None:
        if (
            not isinstance(token, str)
            or not 16 <= len(token) <= 761
            or any(not 0x21 <= ord(character) <= 0x7E for character in token)
        ):
            raise ValueError("credential must be 16-761 visible ASCII characters")
        self.__token = token

    def _metadata(self) -> tuple[tuple[str, str], ...]:
        """Return transport metadata. This is internal to the secure channel adapter."""

        return (("authorization", f"Bearer {self.__token}"),)

    def __repr__(self) -> str:
        return "StaticBearerCredential([REDACTED])"

    __str__ = __repr__

    def __getstate__(self) -> NoReturn:
        raise TypeError("credentials cannot be serialized")
