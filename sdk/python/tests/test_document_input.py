import unittest

from colossus.api.v1alpha1 import system_pb2


class DocumentInputTests(unittest.TestCase):
    def test_model_formats_round_trip_and_legacy_absence_is_empty(self) -> None:
        profile = system_pb2.ConfiguredModelProfile(
            profile="office",
            model="configured-model",
            provider="open_ai_responses",
            provider_profile="work",
            document_media_types=[
                "application/pdf",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ],
        )
        info = system_pb2.ServerInfo(
            capabilities=[system_pb2.Capability(name="attachments.document_input", enabled=True)],
            model_profiles=[profile],
        )
        self.assertEqual(system_pb2.ServerInfo.FromString(info.SerializeToString()), info)
        legacy = system_pb2.ConfiguredModelProfile(profile="legacy")
        self.assertEqual(
            list(
                system_pb2.ConfiguredModelProfile.FromString(
                    legacy.SerializeToString()
                ).document_media_types
            ),
            [],
        )
        self.assertEqual(profile.DESCRIPTOR.fields_by_name["document_media_types"].number, 7)
