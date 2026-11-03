use colossus_api::CallerContext;
use colossus_api_proto::v1alpha1::ConfiguredModelProfile;
use colossus_grpc::ModelCatalogProvider;
use colossus_runtime::Runtime;
use std::sync::Arc;

pub(super) struct RuntimeModelCatalog(pub(super) Arc<Runtime>);

impl ModelCatalogProvider for RuntimeModelCatalog {
    fn model_profiles(&self, caller: &CallerContext) -> Vec<ConfiguredModelProfile> {
        let defaults: Vec<_> = caller
            .principal()
            .allowed_roles()
            .take(64)
            .filter_map(|role| {
                self.0
                    .provider_route(role)
                    .ok()
                    .map(|route| (role, route.model_profile))
            })
            .collect();
        self.0
            .model_profiles()
            .into_iter()
            .filter(|route| !route.model_profile.is_empty() && route.model_profile.len() <= 128)
            .take(256)
            .map(|route| ConfiguredModelProfile {
                document_media_types: self
                    .0
                    .configured_model_document_media_types(&route.model_profile)
                    .unwrap_or_default(),
                default_roles: defaults
                    .iter()
                    .filter(|(_, profile)| *profile == route.model_profile)
                    .map(|(role, _)| (*role).into())
                    .collect(),
                profile: route.model_profile,
                model: route.model.chars().take(256).collect(),
                provider: route.provider.chars().take(128).collect(),
                provider_profile: route.provider_profile.chars().take(128).collect(),
                reasoning_effort: route.reasoning_effort.map(|effort| effort.as_str().into()),
            })
            .collect()
    }
}
