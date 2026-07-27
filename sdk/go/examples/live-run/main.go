// Command live-run connects the Go SDK with a one-use credential from stdin.
package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"time"

	colossus "github.com/obscuritylabs/colossus/sdk/go"
	durablerun "github.com/obscuritylabs/colossus/sdk/go/examples/durable-run"
	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
)

const maxCredentialBytes = 761

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) != 6 {
		return errors.New(
			"usage: live-run DESCRIPTOR CERTIFICATE INSTANCE_ID CERTIFICATE_SHA256 PROMPT",
		)
	}
	credential, err := readPipeCredential()
	if err != nil {
		return err
	}
	descriptorJSON, err := os.ReadFile(os.Args[1])
	if err != nil {
		return err
	}
	descriptor, err := colossus.ParseEndpointDescriptor(descriptorJSON)
	if err != nil {
		return err
	}
	certificate, err := os.ReadFile(os.Args[2])
	if err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	connection, err := colossus.Dial(
		ctx,
		descriptor,
		certificate,
		os.Args[3],
		os.Args[4],
		v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
		credential,
	)
	if err != nil {
		return err
	}
	defer connection.Close()
	result, err := durablerun.RunPrompt(
		ctx,
		v1alpha1.NewAgentRunServiceClient(connection),
		os.Args[5],
		v1alpha1.RunMode_RUN_MODE_EXECUTE,
		nil,
	)
	if err != nil {
		return err
	}
	fmt.Println(result.Output)
	return nil
}

func readPipeCredential() (*colossus.StaticBearerCredential, error) {
	info, err := os.Stdin.Stat()
	if err != nil {
		return nil, err
	}
	if info.Mode()&os.ModeNamedPipe == 0 && info.Mode()&os.ModeSocket == 0 {
		return nil, errors.New(
			"the live SDK credential must arrive through an anonymous pipe",
		)
	}
	raw, err := io.ReadAll(io.LimitReader(os.Stdin, maxCredentialBytes+1))
	if err != nil {
		return nil, err
	}
	if len(raw) == 0 || len(raw) > maxCredentialBytes {
		return nil, errors.New("the live SDK credential is invalid")
	}
	return colossus.NewStaticBearerCredential(string(raw))
}
