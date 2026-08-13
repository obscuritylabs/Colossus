package colossus

import (
	"bytes"
	"context"
	"errors"
	"time"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
)

const apiPackage = "colossus.api.v1alpha1"

func validateServerInfo(
	serverInfo *v1alpha1.ServerInfo,
	expectedInstanceID string,
	expectedDeploymentMode v1alpha1.DeploymentMode,
) error {
	if expectedDeploymentMode != v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON &&
		expectedDeploymentMode != v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SIDECAR {
		return errors.New("expected deployment mode must be shared_daemon or sidecar")
	}
	if serverInfo == nil ||
		serverInfo.InstanceId != expectedInstanceID ||
		serverInfo.DeploymentMode != expectedDeploymentMode {
		return errors.New("authenticated Colossus server identity is incompatible")
	}
	for _, advertised := range serverInfo.ApiPackages {
		if advertised == apiPackage {
			return nil
		}
	}
	return errors.New("authenticated Colossus server identity is incompatible")
}

// Dial creates a secure local gRPC connection without ambient credential discovery.
//
// Generic retries are disabled because the SDK cannot infer whether an effectful RPC
// is safe to replay. Use RunWatcher for explicit read-only watch reconnection.
func Dial(
	ctx context.Context,
	descriptor EndpointDescriptor,
	leafCertificatePEM []byte,
	expectedInstanceID string,
	expectedCertificateSHA256 string,
	expectedDeploymentMode v1alpha1.DeploymentMode,
	credential *StaticBearerCredential,
) (*grpc.ClientConn, error) {
	if credential == nil {
		return nil, errors.New("credential is required")
	}
	target, err := descriptor.target()
	if err != nil {
		return nil, err
	}
	tlsConfig, err := pinnedTLSConfig(
		descriptor,
		bytes.Clone(leafCertificatePEM),
		expectedInstanceID,
		expectedCertificateSHA256,
	)
	if err != nil {
		return nil, err
	}

	connection, err := grpc.NewClient(
		target,
		grpc.WithTransportCredentials(credentials.NewTLS(tlsConfig)),
		grpc.WithPerRPCCredentials(credential),
		grpc.WithNoProxy(),
		grpc.WithDisableRetry(),
		grpc.WithDisableServiceConfig(),
		grpc.WithUserAgent("colossus-go-sdk/0.10.8"),
		grpc.WithDefaultCallOptions(
			grpc.MaxCallRecvMsgSize(4*1024*1024),
			grpc.MaxCallSendMsgSize(4*1024*1024),
		),
	)
	if err != nil {
		return nil, err
	}
	verificationContext, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	response, err := v1alpha1.NewSystemServiceClient(connection).GetServerInfo(
		verificationContext,
		&v1alpha1.GetServerInfoRequest{},
	)
	if err == nil {
		err = validateServerInfo(
			response.GetServerInfo(),
			expectedInstanceID,
			expectedDeploymentMode,
		)
	}
	if err != nil {
		_ = connection.Close()
		return nil, err
	}
	return connection, nil
}
