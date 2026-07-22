package colossus

import (
	"bytes"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"io"
	"net"
	"net/url"
	"regexp"
	"strconv"
)

var (
	certificatePinPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)
	instanceIDPattern     = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)
	endpointPattern       = regexp.MustCompile(`^https://(?:127\.0\.0\.1|\[::1\]):[1-9][0-9]{0,4}/?$`)
)

// EndpointDescriptor is credential-free, owner-readable connection metadata.
type EndpointDescriptor struct {
	SchemaVersion     int    `json:"schema_version"`
	APIVersion        string `json:"api_version"`
	InstanceID        string `json:"instance_id"`
	Endpoint          string `json:"endpoint"`
	PID               uint32 `json:"pid"`
	CertificateSHA256 string `json:"certificate_sha256"`
}

// ParseEndpointDescriptor rejects unknown fields so a token cannot be smuggled into it.
func ParseEndpointDescriptor(data []byte) (EndpointDescriptor, error) {
	if len(data) > 4096 {
		return EndpointDescriptor{}, errors.New("endpoint descriptor exceeds the size limit")
	}
	var descriptor EndpointDescriptor
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&descriptor); err != nil {
		return EndpointDescriptor{}, errors.New("endpoint descriptor is invalid JSON")
	}
	var trailing any
	if err := decoder.Decode(&trailing); !errors.Is(err, io.EOF) {
		return EndpointDescriptor{}, errors.New("endpoint descriptor has trailing data")
	}
	if err := descriptor.Validate(); err != nil {
		return EndpointDescriptor{}, err
	}
	return descriptor, nil
}

// Validate enforces the local TLS-only public transport boundary.
func (descriptor EndpointDescriptor) Validate() error {
	if descriptor.SchemaVersion != 1 {
		return errors.New("unsupported endpoint descriptor schema_version")
	}
	if descriptor.APIVersion != "colossus.api.v1alpha1" {
		return errors.New("unsupported endpoint descriptor api_version")
	}
	if err := validateBoundedText("instance_id", descriptor.InstanceID, 128); err != nil {
		return err
	}
	if !instanceIDPattern.MatchString(descriptor.InstanceID) ||
		descriptor.InstanceID == "00000000-0000-0000-0000-000000000000" {
		return errors.New("instance_id must be a canonical non-nil UUID")
	}
	if len(descriptor.Endpoint) == 0 || len(descriptor.Endpoint) > 256 {
		return errors.New("endpoint must be a non-empty bounded string")
	}
	if descriptor.PID == 0 {
		return errors.New("pid must be a nonzero unsigned 32-bit integer")
	}
	if !certificatePinPattern.MatchString(descriptor.CertificateSHA256) {
		return errors.New("certificate_sha256 must be 64 lowercase hexadecimal digits")
	}
	_, err := descriptor.target()
	return err
}

func validateBoundedText(field string, value string, maximum int) error {
	if len(value) == 0 || len(value) > maximum {
		return errors.New(field + " must be a non-empty bounded string")
	}
	for _, character := range value {
		if character < 32 || character == 127 {
			return errors.New(field + " contains a control character")
		}
	}
	return nil
}

func (descriptor EndpointDescriptor) target() (string, error) {
	if !endpointPattern.MatchString(descriptor.Endpoint) {
		return "", errors.New(
			"endpoint must be a canonical credential-free https literal-loopback URL",
		)
	}
	endpoint, err := url.Parse(descriptor.Endpoint)
	if err != nil ||
		endpoint.Scheme != "https" ||
		endpoint.Opaque != "" ||
		endpoint.User != nil ||
		endpoint.RawQuery != "" ||
		endpoint.Fragment != "" ||
		(endpoint.Path != "" && endpoint.Path != "/") {
		return "", errors.New(
			"endpoint must be a credential-free https URL without path, query, or fragment",
		)
	}
	host := endpoint.Hostname()
	if host != "127.0.0.1" && host != "::1" {
		return "", errors.New("endpoint host must be a literal loopback address")
	}
	portText := endpoint.Port()
	port, err := strconv.Atoi(portText)
	if err != nil || port < 1 || port > 65535 {
		return "", errors.New("endpoint must contain a valid explicit port")
	}
	return net.JoinHostPort(host, portText), nil
}

func parsePinnedLeaf(leafCertificatePEM []byte) (*x509.Certificate, error) {
	if len(leafCertificatePEM) > 65536 {
		return nil, errors.New("leaf certificate exceeds the size limit")
	}
	trimmedPEM := bytes.TrimSpace(leafCertificatePEM)
	if !bytes.HasPrefix(trimmedPEM, []byte("-----BEGIN CERTIFICATE-----")) {
		return nil, errors.New("exactly one public leaf certificate is required")
	}
	block, remainder := pem.Decode(trimmedPEM)
	if block == nil ||
		block.Type != "CERTIFICATE" ||
		len(block.Headers) != 0 ||
		len(bytes.TrimSpace(remainder)) != 0 {
		return nil, errors.New("exactly one public leaf certificate is required")
	}
	certificate, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return nil, errors.New("leaf certificate is invalid")
	}
	if !certificate.BasicConstraintsValid || certificate.IsCA {
		return nil, errors.New(
			"endpoint identity certificate must declare BasicConstraints CA=false",
		)
	}
	return certificate, nil
}

// CertificateSHA256 returns the lowercase digest of a single leaf's DER bytes.
func CertificateSHA256(leafCertificatePEM []byte) (string, error) {
	certificate, err := parsePinnedLeaf(leafCertificatePEM)
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256(certificate.Raw)
	return hex.EncodeToString(digest[:]), nil
}

func pinnedTLSConfig(
	descriptor EndpointDescriptor,
	leafCertificatePEM []byte,
	expectedInstanceID string,
	expectedCertificateSHA256 string,
) (*tls.Config, error) {
	if err := descriptor.Validate(); err != nil {
		return nil, err
	}
	if !instanceIDPattern.MatchString(expectedInstanceID) ||
		expectedInstanceID == "00000000-0000-0000-0000-000000000000" {
		return nil, errors.New(
			"independently provisioned instance ID must be a canonical non-nil UUID",
		)
	}
	if subtle.ConstantTimeCompare(
		[]byte(descriptor.InstanceID),
		[]byte(expectedInstanceID),
	) != 1 {
		return nil, errors.New(
			"endpoint descriptor instance ID does not match the independently provisioned identity",
		)
	}
	if !certificatePinPattern.MatchString(expectedCertificateSHA256) {
		return nil, errors.New(
			"independently provisioned certificate pin must be 64 lowercase hexadecimal digits",
		)
	}
	expectedPin, err := hex.DecodeString(expectedCertificateSHA256)
	if err != nil {
		return nil, errors.New("independently provisioned certificate pin is invalid")
	}
	descriptorPin, err := hex.DecodeString(descriptor.CertificateSHA256)
	if err != nil ||
		subtle.ConstantTimeCompare(descriptorPin, expectedPin) != 1 {
		return nil, errors.New(
			"endpoint descriptor certificate pin does not match the independently provisioned pin",
		)
	}
	certificate, err := parsePinnedLeaf(leafCertificatePEM)
	if err != nil {
		return nil, err
	}
	actualPin := sha256.Sum256(certificate.Raw)
	if subtle.ConstantTimeCompare(actualPin[:], expectedPin) != 1 {
		return nil, errors.New(
			"leaf certificate does not match the independently provisioned pin",
		)
	}

	roots := x509.NewCertPool()
	roots.AddCert(certificate)
	endpoint, _ := url.Parse(descriptor.Endpoint)

	// Standard verification runs first. VerifyConnection then binds the actual peer
	// leaf to the descriptor, even if a future certificate shape could build another
	// chain to the supplied trust anchor.
	return &tls.Config{
		MinVersion: tls.VersionTLS13,
		RootCAs:    roots,
		ServerName: endpoint.Hostname(),
		VerifyConnection: func(state tls.ConnectionState) error {
			if len(state.PeerCertificates) == 0 {
				return errors.New("Colossus peer did not present a certificate")
			}
			peerPin := sha256.Sum256(state.PeerCertificates[0].Raw)
			if subtle.ConstantTimeCompare(peerPin[:], expectedPin) != 1 {
				return errors.New("Colossus peer certificate pin mismatch")
			}
			return nil
		},
	}, nil
}
