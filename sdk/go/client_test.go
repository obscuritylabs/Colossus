package colossus

import (
	"context"
	"crypto/tls"
	"net"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

const expectedTestInstanceID = "00000000-0000-4000-8000-000000000001"

type connectorSystemService struct {
	v1alpha1.UnimplementedSystemServiceServer
	authenticatedCalls *atomic.Int32
}

func (service *connectorSystemService) GetServerInfo(
	ctx context.Context,
	_ *v1alpha1.GetServerInfoRequest,
) (*v1alpha1.GetServerInfoResponse, error) {
	requestMetadata, ok := metadata.FromIncomingContext(ctx)
	if !ok {
		return nil, status.Error(codes.Unauthenticated, "missing request metadata")
	}
	authorization := requestMetadata.Get("authorization")
	if len(authorization) != 1 || authorization[0] != "Bearer connector-test-token" {
		return nil, status.Error(codes.Unauthenticated, "invalid bearer credential")
	}
	service.authenticatedCalls.Add(1)
	return &v1alpha1.GetServerInfoResponse{
		ServerInfo: compatibleServerInfo(),
	}, nil
}

func compatibleServerInfo() *v1alpha1.ServerInfo {
	return &v1alpha1.ServerInfo{
		InstanceId:     expectedTestInstanceID,
		ApiPackages:    []string{apiPackage},
		DeploymentMode: v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
	}
}

func TestAuthenticatedServerInfoBindsIdentityAPIAndDeployment(t *testing.T) {
	t.Parallel()
	if err := validateServerInfo(
		compatibleServerInfo(),
		expectedTestInstanceID,
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
	); err != nil {
		t.Fatal(err)
	}

	for name, mutate := range map[string]func(*v1alpha1.ServerInfo){
		"instance": func(info *v1alpha1.ServerInfo) {
			info.InstanceId = "00000000-0000-4000-8000-000000000002"
		},
		"api": func(info *v1alpha1.ServerInfo) {
			info.ApiPackages = []string{"colossus.api.v2"}
		},
		"deployment": func(info *v1alpha1.ServerInfo) {
			info.DeploymentMode = v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SIDECAR
		},
	} {
		t.Run(name, func(t *testing.T) {
			info := compatibleServerInfo()
			mutate(info)
			if err := validateServerInfo(
				info,
				expectedTestInstanceID,
				v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
			); err == nil {
				t.Fatal("expected incompatible live identity")
			}
		})
	}
}

func TestGRPCConnectorRejectsEmbeddedOrUnspecifiedExpectation(t *testing.T) {
	t.Parallel()
	for _, mode := range []v1alpha1.DeploymentMode{
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_UNSPECIFIED,
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_EMBEDDED,
	} {
		if err := validateServerInfo(
			compatibleServerInfo(),
			expectedTestInstanceID,
			mode,
		); err == nil {
			t.Fatalf("expected deployment mode %v to fail", mode)
		}
	}
}

func TestConnectorVerifiesPinnedTLSBearerAndLiveIdentity(t *testing.T) {
	certificatePEM, err := os.ReadFile(filepath.Join("..", "testdata", "connector-cert.pem"))
	if err != nil {
		t.Fatal(err)
	}
	privateKeyPEM, err := os.ReadFile(filepath.Join("..", "testdata", "connector-key.pem"))
	if err != nil {
		t.Fatal(err)
	}
	serverCertificate, err := tls.X509KeyPair(certificatePEM, privateKeyPEM)
	if err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	authenticatedCalls := &atomic.Int32{}
	server := grpc.NewServer(grpc.Creds(credentials.NewTLS(&tls.Config{
		Certificates: []tls.Certificate{serverCertificate},
		MinVersion:   tls.VersionTLS13,
	})))
	v1alpha1.RegisterSystemServiceServer(server, &connectorSystemService{
		authenticatedCalls: authenticatedCalls,
	})
	go func() {
		_ = server.Serve(listener)
	}()
	t.Cleanup(server.Stop)

	pin, err := CertificateSHA256(certificatePEM)
	if err != nil {
		t.Fatal(err)
	}
	descriptor := EndpointDescriptor{
		SchemaVersion:     1,
		APIVersion:        apiPackage,
		InstanceID:        expectedTestInstanceID,
		Endpoint:          "https://" + listener.Addr().String(),
		PID:               1,
		CertificateSHA256: pin,
	}
	credential, err := NewStaticBearerCredential("connector-test-token")
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	connection, err := Dial(
		ctx,
		descriptor,
		certificatePEM,
		expectedTestInstanceID,
		pin,
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
		credential,
	)
	if err != nil {
		t.Fatal(err)
	}
	if got := authenticatedCalls.Load(); got != 1 {
		t.Fatalf("authenticated GetServerInfo calls = %d, want 1", got)
	}
	if err := connection.Close(); err != nil {
		t.Fatal(err)
	}

	wrongLeafPEM, err := os.ReadFile(filepath.Join("..", "testdata", "leaf.pem"))
	if err != nil {
		t.Fatal(err)
	}
	wrongPin, err := CertificateSHA256(wrongLeafPEM)
	if err != nil {
		t.Fatal(err)
	}
	wrongDescriptor := descriptor
	wrongDescriptor.CertificateSHA256 = wrongPin
	wrongCredential, err := NewStaticBearerCredential("wrong-leaf-token-1")
	if err != nil {
		t.Fatal(err)
	}
	wrongContext, wrongCancel := context.WithTimeout(context.Background(), time.Second)
	defer wrongCancel()
	if connection, err := Dial(
		wrongContext,
		wrongDescriptor,
		wrongLeafPEM,
		expectedTestInstanceID,
		wrongPin,
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
		wrongCredential,
	); err == nil {
		_ = connection.Close()
		t.Fatal("expected connector to reject an unrelated trusted leaf")
	}
	if got := authenticatedCalls.Load(); got != 1 {
		t.Fatalf("authenticated calls after wrong-leaf attempt = %d, want 1", got)
	}
}
