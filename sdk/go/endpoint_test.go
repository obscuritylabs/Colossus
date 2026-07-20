package colossus

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"net"
	"testing"
	"time"
)

func validDescriptorJSON(endpoint string) []byte {
	data, _ := json.Marshal(map[string]any{
		"schema_version":     1,
		"api_version":        "colossus.api.v1alpha1",
		"instance_id":        "00000000-0000-4000-8000-000000000001",
		"endpoint":           endpoint,
		"pid":                4242,
		"certificate_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	})
	return data
}

func TestDescriptorRequiresLiteralLoopbackTLS(t *testing.T) {
	t.Parallel()
	descriptor, err := ParseEndpointDescriptor(
		validDescriptorJSON("https://127.0.0.1:43119"),
	)
	if err != nil {
		t.Fatal(err)
	}
	target, err := descriptor.target()
	if err != nil {
		t.Fatal(err)
	}
	if target != "127.0.0.1:43119" {
		t.Fatalf("unexpected target %q", target)
	}

	for _, endpoint := range []string{
		"https://example.com:43119",
		"http://127.0.0.1:43119",
		"https://user:pass@127.0.0.1:43119",
		"https://localhost:43119",
	} {
		if _, err := ParseEndpointDescriptor(validDescriptorJSON(endpoint)); err == nil {
			t.Fatalf("expected endpoint %q to be rejected", endpoint)
		}
	}
}

func TestDescriptorRejectsTokenField(t *testing.T) {
	t.Parallel()
	data := []byte(`{
		"schema_version": 1,
		"api_version": "colossus.api.v1alpha1",
		"instance_id": "00000000-0000-4000-8000-000000000001",
		"endpoint": "https://127.0.0.1:43119",
		"pid": 4242,
		"certificate_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"bearer_token": "cls_v1.should-never-be-here"
	}`)
	if _, err := ParseEndpointDescriptor(data); err == nil {
		t.Fatal("expected descriptor token field to be rejected")
	}
	data = []byte(`{
		"schema_version": 1,
		"api_version": "colossus.api.v1alpha1",
		"instance_id": "00000000-0000-4000-8000-000000000001",
		"endpoint": "https://127.0.0.1:43119",
		"pid": 4242,
		"certificate_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		"server_name": "localhost"
	}`)
	if _, err := ParseEndpointDescriptor(data); err == nil {
		t.Fatal("expected unknown server_name field to be rejected")
	}
}

func TestDescriptorJSONIsBoundedBeforeParsing(t *testing.T) {
	t.Parallel()
	if _, err := ParseEndpointDescriptor(make([]byte, 4097)); err == nil {
		t.Fatal("expected oversized descriptor to be rejected")
	}
}

func TestDescriptorRejectsNoncanonicalValues(t *testing.T) {
	t.Parallel()
	for _, endpoint := range []string{
		"https://127.1:43119",
		"https://127.0.0.1:043119",
		"https://[0:0:0:0:0:0:0:1]:43119",
	} {
		if _, err := ParseEndpointDescriptor(validDescriptorJSON(endpoint)); err == nil {
			t.Fatalf("expected noncanonical endpoint %q to be rejected", endpoint)
		}
	}

	var descriptor map[string]any
	if err := json.Unmarshal(validDescriptorJSON("https://127.0.0.1:43119"), &descriptor); err != nil {
		t.Fatal(err)
	}
	descriptor["certificate_sha256"] = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
	data, _ := json.Marshal(descriptor)
	if _, err := ParseEndpointDescriptor(data); err == nil {
		t.Fatal("expected uppercase certificate pin to be rejected")
	}
	descriptor["certificate_sha256"] = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	descriptor["instance_id"] = "00000000-0000-0000-0000-000000000000"
	data, _ = json.Marshal(descriptor)
	if _, err := ParseEndpointDescriptor(data); err == nil {
		t.Fatal("expected nil instance UUID to be rejected")
	}
}

func TestTLSConfigPinsActualLeaf(t *testing.T) {
	t.Parallel()
	leafPEM, pin := selfSignedLeaf(t)
	descriptor := EndpointDescriptor{
		SchemaVersion:     1,
		APIVersion:        "colossus.api.v1alpha1",
		InstanceID:        "00000000-0000-4000-8000-000000000001",
		Endpoint:          "https://127.0.0.1:43119",
		PID:               4242,
		CertificateSHA256: pin,
	}
	config, err := pinnedTLSConfig(descriptor, leafPEM, descriptor.InstanceID, pin)
	if err != nil {
		t.Fatal(err)
	}
	if config.MinVersion != 0x0304 {
		t.Fatal("TLS 1.3 minimum was not enforced")
	}

	descriptor.CertificateSHA256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	if _, err := pinnedTLSConfig(descriptor, leafPEM, descriptor.InstanceID, pin); err == nil {
		t.Fatal("expected descriptor pin mismatch")
	}

	descriptor.CertificateSHA256 = pin
	if _, err := pinnedTLSConfig(
		descriptor,
		leafPEM,
		descriptor.InstanceID,
		"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	); err == nil {
		t.Fatal("expected independent pin mismatch")
	}
	if _, err := pinnedTLSConfig(
		descriptor,
		leafPEM,
		descriptor.InstanceID,
		"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
	); err == nil {
		t.Fatal("expected malformed independent pin")
	}
	if _, err := pinnedTLSConfig(
		descriptor,
		leafPEM,
		"00000000-0000-4000-8000-000000000002",
		pin,
	); err == nil {
		t.Fatal("expected independent instance mismatch")
	}

	leadingJunk := append([]byte("not a PEM block\n"), leafPEM...)
	if _, err := CertificateSHA256(leadingJunk); err == nil {
		t.Fatal("expected leading certificate input to be rejected")
	}
	twoCertificates := append(append([]byte{}, leafPEM...), leafPEM...)
	if _, err := CertificateSHA256(twoCertificates); err == nil {
		t.Fatal("expected multiple certificates to be rejected")
	}
	if _, err := CertificateSHA256(make([]byte, 65537)); err == nil {
		t.Fatal("expected oversized certificate input to be rejected")
	}
	missingConstraintsPEM, _ := selfSignedLeafWithConstraints(t, false)
	if _, err := CertificateSHA256(missingConstraintsPEM); err == nil {
		t.Fatal("expected missing BasicConstraints to be rejected")
	}
}

func selfSignedLeaf(t *testing.T) ([]byte, string) {
	return selfSignedLeafWithConstraints(t, true)
}

func selfSignedLeafWithConstraints(t *testing.T, basicConstraintsValid bool) ([]byte, string) {
	t.Helper()
	privateKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "Colossus test"},
		NotBefore:             time.Now().Add(-time.Minute),
		NotAfter:              time.Now().Add(time.Hour),
		IPAddresses:           []net.IP{net.ParseIP("127.0.0.1")},
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: basicConstraintsValid,
		IsCA:                  false,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &privateKey.PublicKey, privateKey)
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(der)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), hex.EncodeToString(digest[:])
}
