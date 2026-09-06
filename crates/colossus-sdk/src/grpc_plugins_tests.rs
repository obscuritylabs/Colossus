use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, ExtensionApi, RequestId, scopes,
};
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

fn png_url(image: &DynamicImage) -> String {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("PNG fixture");
    format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    )
}

fn example(icon: String) -> proto::AgentPlugin {
    proto::AgentPlugin {
        name: "example".into(),
        icon_data_url: icon,
        trust: Some(proto::PluginTrust::default()),
        ..Default::default()
    }
}

fn decode_plugin(value: proto::AgentPlugin) -> ApiResult<PluginInventoryEntry> {
    plugin_from_proto(value, &mut icons::NormalizationBudget::default())
}

#[test]
fn discovery_preserves_valid_icons_and_accepts_older_servers_without_them() {
    let icon = png_url(&DynamicImage::new_rgba8(16, 16));
    for icon in [String::new(), icon] {
        let entry = decode_plugin(example(icon.clone())).expect("discovery");
        assert_eq!(entry.icon_data_url, (!icon.is_empty()).then_some(icon));
    }
}

#[test]
fn external_icons_reject_remote_sources_malformed_images_and_unbounded_payloads() {
    for icon in [
        "https://tracker.example/icon".into(),
        "file:///private/icon.png".into(),
        "data:image/svg+xml;base64,PHN2Zy8+".into(),
        "data:image/png;base64,not-base64".into(),
        "data:image/png;base64,iVBORw0KGgo=".into(),
        format!("data:image/png;base64,{}", "A".repeat(87_388)),
        png_url(&DynamicImage::new_rgba8(513, 1)),
    ] {
        assert!(decode_plugin(example(icon)).is_err());
    }
}

#[test]
fn external_pngs_are_normalized_before_sdk_release() {
    let icon = png_url(&DynamicImage::new_rgba8(16, 16));
    let mut bytes = STANDARD
        .decode(icon.strip_prefix("data:image/png;base64,").expect("prefix"))
        .expect("PNG");
    bytes.extend_from_slice(b"private trailing metadata");
    let source = format!("data:image/png;base64,{}", STANDARD.encode(&bytes));
    assert_eq!(
        decode_plugin(example(source))
            .expect("normalized")
            .icon_data_url,
        Some(icon)
    );
    bytes.resize(64 * 1024 + 1, 0);
    let oversized = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
    assert!(oversized.len() <= 87_406);
    assert!(decode_plugin(example(oversized)).is_err());
}

#[test]
fn discovery_keeps_the_aggregate_metadata_limit() {
    let mut budget = InventoryBudget::default();
    let mut output = Vec::new();
    for _ in 0..7 {
        let mut plugin = example(String::new());
        plugin.description = "x".repeat(1024 * 1024);
        budget
            .append(
                &mut output,
                proto::ListExtensionsResponse {
                    plugins: vec![plugin],
                    ..Default::default()
                },
            )
            .expect("metadata within bound");
    }
    let mut plugin = example(String::new());
    plugin.description = "x".repeat(1024 * 1024);
    assert!(
        budget
            .append(
                &mut output,
                proto::ListExtensionsResponse {
                    plugins: vec![plugin],
                    ..Default::default()
                }
            )
            .is_err()
    );
}

#[test]
fn compressed_catalogs_bound_pixel_and_image_work_across_pages() {
    for (side, expected_icons) in [(512, 32), (1, 64)] {
        let icon = png_url(&DynamicImage::new_rgba8(side, side));
        assert!(icon.len() * 1000 < MAX_CATALOG_RESPONSE_BYTES);
        let mut budget = InventoryBudget::default();
        let mut output = Vec::new();
        for page in 0..50 {
            budget
                .append(
                    &mut output,
                    proto::ListExtensionsResponse {
                        plugins: (0..20)
                            .map(|index| {
                                let mut plugin = example(icon.clone());
                                plugin.name = format!("plugin-{}", page * 20 + index);
                                plugin
                            })
                            .collect(),
                        ..Default::default()
                    },
                )
                .expect("metadata survives exhausted normalization budget");
        }
        assert_eq!(output.len(), 1000);
        assert_eq!(
            output
                .iter()
                .filter(|plugin| plugin.icon_data_url.is_some())
                .count(),
            expected_icons
        );
        for (index, plugin) in output.iter().enumerate() {
            assert_eq!(plugin.manifest.name, format!("plugin-{index}"));
        }
    }
}

struct Catalog {
    entries: Vec<PluginInventoryEntry>,
    pages: AtomicUsize,
}

#[async_trait]
impl ExtensionApi for Catalog {
    async fn plugins(&self, _: &CallerContext, _: bool) -> ApiResult<Vec<PluginInventoryEntry>> {
        self.pages.fetch_add(1, Ordering::Relaxed);
        Ok(self.entries.clone())
    }

    async fn skill(&self, _: &CallerContext, _: &str, _: &str) -> ApiResult<PluginSkillContent> {
        unreachable!("inventory does not load instructions")
    }

    async fn resources(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
    ) -> ApiResult<Vec<PluginResourceEntry>> {
        unreachable!("inventory does not load resources")
    }

    async fn resource(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
        _: &str,
    ) -> ApiResult<PluginResourceRead> {
        unreachable!("inventory does not load resources")
    }
}

struct Credential;

#[async_trait]
impl crate::CredentialProvider for Credential {
    async fn load(&self) -> crate::SdkResult<crate::Secret> {
        crate::Secret::new(b"plugin-catalog-test".to_vec())
    }
}

fn large_icon() -> String {
    // Incompressible pixels make 100 valid PNG data URLs exceed the old 8 MiB
    // aggregate bound. This drives the actual SDK collector and server pagination.
    let mut random = 1_u32;
    let pixels = (0..127 * 127 * 4)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random.to_le_bytes()[0]
        })
        .collect();
    let icon = png_url(&DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(127, 127, pixels).expect("pixels"),
    ));
    assert!(icon.len() <= 87_406);
    assert!(icon.len() * 100 > MAX_CATALOG_METADATA_BYTES);
    icon
}

#[test]
fn discovery_bounds_total_wire_bytes_even_when_extra_icons_are_discarded() {
    let icon = large_icon();
    let mut budget = InventoryBudget::default();
    let mut output = Vec::new();
    for page in 0..7 {
        let result = budget.append(
            &mut output,
            proto::ListExtensionsResponse {
                plugins: (0..20).map(|_| example(icon.clone())).collect(),
                ..Default::default()
            },
        );
        assert_eq!(result.is_ok(), page < 6);
    }
    assert_eq!(output.len(), 120);
    assert!(budget.icon_bytes <= MAX_CATALOG_ICON_BYTES);
}

#[tokio::test]
async fn grpc_sdk_discovers_large_icon_catalogs_across_real_transport_pages() {
    let icon = large_icon();
    let entries = (0..100)
        .map(|index| {
            let mut plugin = decode_plugin(example(icon.clone())).expect("valid icon");
            plugin.manifest.name = format!("plugin-{index:03}");
            plugin.available = true;
            plugin.status = PluginStatus::Enabled;
            plugin
        })
        .collect::<Vec<_>>();
    let expected = entries
        .iter()
        .map(|plugin| plugin.manifest.name.clone())
        .collect::<Vec<_>>();
    let catalog = Arc::new(Catalog {
        entries,
        pages: AtomicUsize::new(0),
    });
    let adapter = colossus_grpc::ExtensionServiceAdapter::new(Some(catalog.clone()));
    let service = proto::extension_service_server::ExtensionServiceServer::with_interceptor(
        adapter,
        |mut request: Request<()>| {
            if request
                .metadata()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                != Some("Bearer plugin-catalog-test")
            {
                return Err(Status::unauthenticated("test credential required"));
            }
            request
                .extensions_mut()
                .insert(CallerContext::authenticated(
                    ApplicationPrincipal::authenticated(
                        "app:plugins",
                        "test",
                        ApplicationKind::Enrolled,
                        [ApiScope::new(scopes::EXTENSIONS_READ).expect("scope")],
                        ["primary".into()],
                        Vec::<String>::new(),
                    )
                    .expect("principal"),
                    RequestId::new("catalog").expect("request"),
                ));
            Ok(request)
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("address");
    let incoming = Box::pin(futures::stream::try_unfold(
        listener,
        |listener| async move {
            let (stream, _) = listener.accept().await?;
            Ok::<_, std::io::Error>(Some((stream, listener)))
        },
    ));
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            }),
    );
    let channel = Channel::from_shared(format!("http://{address}"))
        .expect("endpoint")
        .connect()
        .await
        .expect("channel");
    let client = GrpcPluginClient {
        transport: Arc::new(GrpcArtifactClient {
            channel,
            credential_provider: Arc::new(Credential),
            closed: watch::channel(false).0,
        }),
    };
    let result = tokio::time::timeout(Duration::from_secs(30), client.list()).await;
    drop(client);
    let _ = shutdown.send(());
    server.await.expect("server task").expect("server shutdown");
    let plugins = result
        .expect("bounded discovery")
        .expect("complete inventory");
    assert_eq!(
        plugins
            .iter()
            .map(|plugin| plugin.manifest.name.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(catalog.pages.load(Ordering::Relaxed), 4);
    let retained_icons = plugins
        .iter()
        .filter_map(|plugin| plugin.icon_data_url.as_ref())
        .collect::<Vec<_>>();
    assert!(!retained_icons.is_empty());
    assert!(retained_icons.len() < plugins.len());
    assert!(retained_icons.iter().map(|icon| icon.len()).sum::<usize>() <= MAX_CATALOG_ICON_BYTES);
}
