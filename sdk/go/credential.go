package colossus

import (
	"context"
	"encoding/json"
	"errors"
)

// StaticBearerCredential holds a caller-supplied token only in memory.
//
// It intentionally has no file, environment, command-line, or descriptor loader.
type StaticBearerCredential struct {
	token string
}

// NewStaticBearerCredential validates an opaque bearer token without logging it.
func NewStaticBearerCredential(token string) (*StaticBearerCredential, error) {
	if len(token) < 16 || len(token) > 761 {
		return nil, errors.New("credential must be 16-761 visible ASCII characters")
	}
	for index := 0; index < len(token); index++ {
		if token[index] < 0x21 || token[index] > 0x7e {
			return nil, errors.New("credential must be 16-761 visible ASCII characters")
		}
	}
	return &StaticBearerCredential{token: token}, nil
}

// GetRequestMetadata implements credentials.PerRPCCredentials.
func (credential *StaticBearerCredential) GetRequestMetadata(
	_ context.Context,
	_ ...string,
) (map[string]string, error) {
	if credential == nil || credential.token == "" {
		return nil, errors.New("credential is unavailable")
	}
	return map[string]string{
		"authorization": "Bearer " + credential.token,
	}, nil
}

// RequireTransportSecurity prevents gRPC from attaching this token to plaintext calls.
func (*StaticBearerCredential) RequireTransportSecurity() bool {
	return true
}

func (StaticBearerCredential) String() string {
	return "StaticBearerCredential([REDACTED])"
}

func (StaticBearerCredential) GoString() string {
	return "StaticBearerCredential([REDACTED])"
}

// MarshalJSON prevents accidental secret disclosure through structured logging.
func (StaticBearerCredential) MarshalJSON() ([]byte, error) {
	return json.Marshal("[REDACTED]")
}
