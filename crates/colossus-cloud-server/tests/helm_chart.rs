//! Static Helm acceptance. No Kubernetes API, installation, or upgrade is used.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_cloud_server::config::Config;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn helm(arguments: &[&str], overrides: Value) -> std::process::Output {
    let directory = tempfile::tempdir().unwrap();
    let values = directory.path().join("values.json");
    std::fs::write(&values, serde_json::to_vec(&overrides).unwrap()).unwrap();
    Command::new(std::env::var_os("HELM").unwrap_or_else(|| "helm".into()))
        .current_dir(root())
        .args(arguments)
        .args(["-f", values.to_str().unwrap()])
        .output()
        .expect("Helm is required; run cargo xtask check helm with HELM or PATH configured")
}

fn render(overrides: Value) -> Vec<Value> {
    let output = helm(
        &[
            "template",
            "colossus",
            "deploy/control-plane/chart",
            "--namespace",
            "colossus",
            "--kube-version",
            "1.34.0",
        ],
        overrides,
    );
    // Rendered output contains synthetic private keys; do not print it on failure.
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_saphyr::from_multiple::<Value>(&String::from_utf8(output.stdout).unwrap())
        .unwrap()
        .into_iter()
        .filter(|value| !value.is_null())
        .collect()
}

fn resource<'a>(documents: &'a [Value], kind: &str, component: &str) -> &'a Value {
    documents
        .iter()
        .find(|value| {
            value["kind"] == kind
                && value["metadata"]["labels"]["app.kubernetes.io/component"] == component
        })
        .unwrap_or_else(|| panic!("missing {kind} for {component}"))
}

fn named<'a>(documents: &'a [Value], kind: &str, name: &str) -> &'a Value {
    documents
        .iter()
        .find(|value| value["kind"] == kind && value["metadata"]["name"] == name)
        .unwrap_or_else(|| panic!("missing {kind} {name}"))
}

fn configuration(documents: &[Value]) -> Config {
    let config = documents
        .iter()
        .find(|v| v["kind"] == "ConfigMap" && v["data"]["control-plane.json"].is_string())
        .unwrap();
    let config: Config =
        serde_json::from_str(config["data"]["control-plane.json"].as_str().unwrap()).unwrap();
    config.validate().unwrap();
    config
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn renders_secure_defaults_and_real_server_configuration() {
    let documents = render(json!({}));
    let config = configuration(&documents);
    assert_eq!(config.public_origin, "https://colossus.example.com");
    assert_eq!(
        config.grpc_endpoint,
        "https://runtimes.colossus.example.com"
    );
    assert!(!config.local_development);
    assert!(config.local_auth.is_some());
    assert!(config.oidc.is_none());
    assert!(config.chat.is_none());
    assert!(matches!(
        config.database.tls,
        colossus_cloud_postgres::CloudDatabaseTls::InCluster { .. }
    ));
    let deployment = resource(&documents, "Deployment", "server");
    let pod = &deployment["spec"]["template"]["spec"];
    assert_eq!(pod["automountServiceAccountToken"], false);
    assert_eq!(
        pod["containers"][0]["securityContext"]["readOnlyRootFilesystem"],
        true
    );
    assert_eq!(pod["topologySpreadConstraints"][0]["minDomains"], 3);
    assert_eq!(
        pod["topologySpreadConstraints"][0]["whenUnsatisfiable"],
        "DoNotSchedule"
    );
    let hpa = documents
        .iter()
        .find(|v| v["kind"] == "HorizontalPodAutoscaler")
        .unwrap();
    assert_eq!(hpa["spec"]["minReplicas"], 3);
    assert_eq!(hpa["spec"]["maxReplicas"], 6);
    let documentation = resource(&documents, "Deployment", "documentation");
    assert_eq!(documentation["spec"]["replicas"], 3);
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        pod["containers"][0]["image"]
            .as_str()
            .unwrap()
            .ends_with(&format!(":v{version}"))
    );
    assert!(
        documentation["spec"]["template"]["spec"]["containers"][0]["image"]
            .as_str()
            .unwrap()
            .ends_with(&format!(":v{version}"))
    );
    let postgres = documents
        .iter()
        .find(|v| v["kind"] == "StatefulSet")
        .unwrap();
    assert_eq!(postgres["spec"]["replicas"], 1);
    assert!(
        !postgres["spec"]["volumeClaimTemplates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let db_secret = named(&documents, "Secret", "colossus-database");
    let uri = STANDARD
        .decode(db_secret["data"]["uri"].as_str().unwrap())
        .unwrap();
    let uri = url::Url::parse(std::str::from_utf8(&uri).unwrap()).unwrap();
    assert_eq!(uri.username(), "colossus");
    assert_eq!(uri.host_str().unwrap(), "colossus-postgres.colossus.svc");
    assert!(documents.iter().any(|v| v["kind"] == "NetworkPolicy"));
    let tls = documents
        .iter()
        .find(|v| v["kind"] == "Secret" && v["data"]["ca.pem"].is_string())
        .unwrap();
    let cert = String::from_utf8(
        STANDARD
            .decode(tls["data"]["ca.pem"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let key = String::from_utf8(
        STANDARD
            .decode(tls["data"]["ca-key.pem"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    rcgen::Issuer::from_ca_cert_pem(&cert, rcgen::KeyPair::from_pem(&key).unwrap()).unwrap();
    let state = documents
        .iter()
        .find(|v| v["kind"] == "Secret" && v["data"]["auth-key"].is_string())
        .unwrap();
    let auth_key = STANDARD
        .decode(state["data"]["auth-key"].as_str().unwrap())
        .unwrap();
    assert_eq!(hex::decode(auth_key).unwrap().len(), 32);
    assert!(
        STANDARD
            .decode(state["data"]["bootstrap-password"].as_str().unwrap())
            .unwrap()
            .len()
            >= 15
    );
    let ingress = documents
        .iter()
        .find(|v| {
            v["kind"] == "Ingress"
                && v["metadata"]["annotations"]["nginx.ingress.kubernetes.io/ssl-passthrough"]
                    == "true"
        })
        .unwrap();
    assert_eq!(
        ingress["spec"]["rules"][0]["http"]["paths"][0]["backend"]["service"]["port"]["name"],
        "tls-connector"
    );
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn advertises_the_external_tls_port_without_changing_service_ports() {
    let documents = render(json!({"routing": {"mode": "gateway-api", "publicPort": 8443}}));
    let config = configuration(&documents);
    assert_eq!(config.public_origin, "https://colossus.example.com:8443");
    assert_eq!(
        config.grpc_endpoint,
        "https://runtimes.colossus.example.com:8443"
    );
    let ports = resource(&documents, "Service", "server")["spec"]["ports"]
        .as_array()
        .unwrap();
    assert!(
        ports
            .iter()
            .any(|port| port["name"] == "http" && port["port"] == 8080)
    );
    assert!(
        ports
            .iter()
            .any(|port| port["name"] == "tls-connector" && port["port"] == 8443)
    );
    for invalid in [json!(0), json!(65536), json!(443.5), json!("8443")] {
        let output = helm(
            &["template", "colossus", "deploy/control-plane/chart"],
            json!({"routing": {"publicPort": invalid}}),
        );
        assert!(!output.status.success(), "invalid public TLS port accepted");
    }
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn renders_shared_hosted_chat_with_private_disposable_caches() {
    for platform in ["kubernetes", "openshift"] {
        let documents = render(json!({
            "platform": platform,
            "postgres": {"enabled": false},
            "database": {"external": {"existingSecret": "database"}},
            "chat": {
                "enabled": true,
                "existingSecret": "chat-state",
                "providerKey": "",
                "maxActiveRuntimes": 4,
                "idleSeconds": 600,
                "anchor": {
                    "endpoint": "https://anchors.example.com/v1/anchors",
                    "existingCaSecret": "anchor-ca"
                }
            }
        }));
        let config = configuration(&documents);
        let mut invalid = config.clone();
        invalid.database.tls = colossus_cloud_postgres::CloudDatabaseTls::InCluster {
            service: "postgres".into(),
            namespace: "colossus".into(),
        };
        assert!(invalid.validate().is_err());
        let chat = config.chat.unwrap();
        assert_eq!(
            chat.home.to_str().unwrap(),
            "/var/lib/colossus-chat/private"
        );
        assert_eq!(chat.max_active_runtimes, 4);
        assert_eq!(chat.idle_seconds, 600);
        let shared = serde_json::to_value(chat.shared.unwrap()).unwrap();
        assert_eq!(shared["keys"]["allow_loopback_http"], false);
        assert_eq!(
            shared["keys"]["anchor_ca_pem_path"],
            "/etc/colossus/chat-anchor/ca.pem"
        );
        let pod = &resource(&documents, "Deployment", "server")["spec"]["template"]["spec"];
        assert_eq!(pod["terminationGracePeriodSeconds"], 120);
        let init = &pod["initContainers"][0];
        assert_eq!(init["image"], pod["containers"][0]["image"]);
        assert_eq!(init["securityContext"]["allowPrivilegeEscalation"], false);
        assert!(init["args"][0].as_str().unwrap().contains("chmod 0700"));
        let volumes = pod["volumes"].as_array().unwrap();
        assert_eq!(
            volumes.iter().find(|v| v["name"] == "chat-cache").unwrap()["emptyDir"]["sizeLimit"],
            "4Gi"
        );
        assert_eq!(
            volumes
                .iter()
                .find(|v| v["name"] == "chat-anchor-ca")
                .unwrap()["secret"]["secretName"],
            "anchor-ca"
        );
        let env = pod["containers"][0]["env"].as_array().unwrap();
        for (variable, key) in [
            ("JOURNAL_KEY", "journal-key"),
            ("SIGNING_KEY", "signing-key"),
            ("WRAPPING_KEY", "wrapping-key"),
            ("ANCHOR_TOKEN", "anchor-token"),
        ] {
            let value = env
                .iter()
                .find(|v| v["name"] == format!("COLOSSUS_CHAT_{variable}"))
                .unwrap();
            assert_eq!(value["valueFrom"]["secretKeyRef"]["name"], "chat-state");
            assert_eq!(value["valueFrom"]["secretKeyRef"]["key"], key);
        }
        assert!(
            !env.iter()
                .any(|v| v["name"] == "COLOSSUS_CHAT_PROVIDER_KEY")
        );
        assert!(
            !documents
                .iter()
                .any(|v| v["kind"] == "PersistentVolumeClaim")
        );
    }
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn renders_routing_mesh_external_database_oidc_and_offline_options() {
    for mode in ["istio", "gateway-api", "none"] {
        for mesh in ["sidecar", "ambient"] {
            let documents = render(json!({"routing":{"mode":mode},"mesh":{"mode":mesh}}));
            configuration(&documents);
            assert!(!documents.iter().any(|v| v["kind"] == "Ingress"));
            if mode == "istio" {
                let vs = documents
                    .iter()
                    .find(|v| v["kind"] == "VirtualService" && v["spec"]["tls"].is_array())
                    .unwrap();
                assert_eq!(
                    vs["spec"]["tls"][0]["route"][0]["destination"]["port"]["number"],
                    8443
                );
            }
            if mode == "gateway-api" {
                let tls = documents.iter().find(|v| v["kind"] == "TLSRoute").unwrap();
                assert_eq!(tls["spec"]["rules"][0]["backendRefs"][0]["port"], 8443);
                assert!(documents.iter().any(|v| v["kind"] == "HTTPRoute"));
            }
            if mesh == "ambient" {
                assert_eq!(
                    resource(&documents, "Deployment", "server")["spec"]["template"]["metadata"]["labels"]
                        ["istio.io/dataplane-mode"],
                    "ambient"
                );
            }
        }
    }
    for provider in ["GitLab", "Keycloak"] {
        for tls in ["webpki_roots", "custom_ca"] {
            let documents = render(json!({
                "postgres":{"enabled":false},
                "database":{"external":{"existingSecret":"rds", "tls":{"mode":tls,"existingSecret":"database-ca"}}},
                "auth":{"local":{"enabled":false},"oidc":{"enabled":true,"label":provider,"issuer":"https://identity.example.com","clientId":"colossus","existingSecret":"oidc"},"bootstrap":{"oidcSubject":"first-admin"},"state":{"existingSecret":"state"}},
                "tls":{"existingSecret":"connector-tls"}
            }));
            let config = configuration(&documents);
            assert!(config.local_auth.is_none());
            assert_eq!(config.oidc.unwrap().label, provider);
            assert!(
                !documents
                    .iter()
                    .any(|v| v["kind"] == "StatefulSet" || v["kind"] == "Secret")
            );
        }
    }
    let documents = render(json!({
        "platform":"openshift", "postgres":{"targetPlatform":"openshift"},
        "global":{"imageRegistry":"mirror.internal","imagePullSecrets":["registry"]},
        "autoscaling":{"enabled":false},"documentation":{"enabled":false},"replicaCount":3
    }));
    configuration(&documents);
    for document in &documents {
        if !matches!(
            document["kind"].as_str(),
            Some("Deployment" | "StatefulSet")
        ) {
            continue;
        }
        let pod = &document["spec"]["template"]["spec"];
        assert!(pod["securityContext"]["runAsUser"].is_null());
        assert!(pod["securityContext"]["fsGroup"].is_null());
        for container in pod["containers"].as_array().unwrap() {
            assert!(container["securityContext"]["runAsUser"].is_null());
            assert!(
                container["image"]
                    .as_str()
                    .unwrap()
                    .starts_with("mirror.internal/")
            );
        }
    }
    assert_eq!(
        resource(&documents, "Deployment", "server")["spec"]["replicas"],
        3
    );
    assert!(
        !documents
            .iter()
            .any(|v| v["kind"] == "HorizontalPodAutoscaler")
    );
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn rejects_invalid_or_unsafe_chart_configuration() {
    for values in [
        json!({"auth":{"local":{"enabled":false}}}),
        json!({"auth":{"oidc":{"enabled":true}}}),
        json!({"auth":{"bootstrap":{"password":"short"}}}),
        json!({"database":{"external":{"tls":{"mode":"disabled"}}}}),
        json!({"postgres":{"enabled":false}}),
        json!({"postgres":{"service":{"type":"LoadBalancer"}}}),
        json!({"postgres":{"namespaceOverride":"other"}}),
        json!({"postgres":{"replicaCount":3}}),
        json!({"browserHost":"same.example.com","connectorHost":"same.example.com"}),
        json!({"autoscaling":{"minReplicas":6,"maxReplicas":3}}),
        json!({"chat":{"enabled":true}}),
        json!({"chat":{"enabled":true,"existingSecret":"state"}}),
        json!({"chat":{"enabled":true,"existingSecret":"state","anchor":{"endpoint":"http://anchors.example.com/v1/anchors"}}}),
        json!({"chat":{"maxActiveRuntimes":65}}),
        json!({"chat":{"enabled":true,"existingSecret":"state","anchor":{"endpoint":"https://anchors.example.com/v1/anchors"}}}),
    ] {
        let output = helm(
            &[
                "template",
                "colossus",
                "deploy/control-plane/chart",
                "--namespace",
                "colossus",
                "--kube-version",
                "1.34.0",
            ],
            values,
        );
        assert!(!output.status.success());
    }
    let output = helm(
        &[
            "template",
            "colossus",
            "deploy/control-plane/chart",
            "--kube-version",
            "1.33.0",
        ],
        json!({}),
    );
    assert!(!output.status.success());
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn packages_vendored_dependencies_without_network_and_binds_the_application_version() {
    use sha2::{Digest, Sha256};
    let chart = root().join("deploy/control-plane/chart");
    let metadata: Value =
        serde_saphyr::from_str(&std::fs::read_to_string(chart.join("Chart.yaml")).unwrap())
            .unwrap();
    assert_eq!(metadata["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata["appVersion"], env!("CARGO_PKG_VERSION"));
    let expected = std::fs::read_to_string(chart.join("DEPENDENCIES.sha256")).unwrap();
    let dependency = std::fs::read(chart.join("charts/postgres-0.21.2.tgz")).unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(dependency)),
        expected.split_whitespace().next().unwrap()
    );
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::var_os("HELM").unwrap_or_else(|| "helm".into()))
        .current_dir(root())
        .args([
            "package",
            "deploy/control-plane/chart",
            "--destination",
            directory.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let archive = directory.path().join(format!(
        "colossus-control-plane-chart-{}.tgz",
        env!("CARGO_PKG_VERSION")
    ));
    let output = helm(
        &[
            "template",
            "offline",
            archive.to_str().unwrap(),
            "--namespace",
            "colossus",
            "--kube-version",
            "1.34.0",
        ],
        json!({}),
    );
    assert!(
        output.status.success(),
        "packaged chart cannot render offline"
    );
}

#[test]
#[ignore = "static Helm tier; cargo xtask check helm"]
fn published_values_examples_render_valid_server_configuration() {
    let examples = root().join("deploy/control-plane/chart/examples");
    for entry in std::fs::read_dir(examples).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml")
            || path.file_name().unwrap() == "gateway-listeners.yaml"
        {
            continue;
        }
        let output = helm(
            &[
                "template",
                "colossus",
                "deploy/control-plane/chart",
                "--namespace",
                "colossus",
                "--kube-version",
                "1.34.0",
                "-f",
                path.to_str().unwrap(),
            ],
            json!({}),
        );
        assert!(
            output.status.success(),
            "values example cannot render: {}",
            path.display()
        );
        let documents =
            serde_saphyr::from_multiple::<Value>(&String::from_utf8(output.stdout).unwrap())
                .unwrap();
        configuration(&documents);
    }
}
