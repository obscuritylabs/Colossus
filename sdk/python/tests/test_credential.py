from __future__ import annotations

import pickle
import unittest

from colossus_sdk.credential import StaticBearerCredential


class CredentialTests(unittest.TestCase):
    def test_representations_are_redacted(self) -> None:
        secret = "cls_v1.credential.very-secret-value"  # noqa: S105
        credential = StaticBearerCredential(secret)

        self.assertNotIn(secret, repr(credential))
        self.assertNotIn(secret, str(credential))
        self.assertEqual(
            credential._metadata(),
            (("authorization", f"Bearer {secret}"),),
        )

    def test_serialization_and_header_injection_are_rejected(self) -> None:
        credential = StaticBearerCredential("cls_v1.credential.very-secret-value")
        with self.assertRaises(TypeError):
            pickle.dumps(credential)
        with self.assertRaises(ValueError):
            StaticBearerCredential("cls_v1.invalid\nheader")
        with self.assertRaises(ValueError):
            StaticBearerCredential("x" * 762)


if __name__ == "__main__":
    unittest.main()
