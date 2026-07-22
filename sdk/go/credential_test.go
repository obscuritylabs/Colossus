package colossus

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
)

func TestCredentialRepresentationsAreRedacted(t *testing.T) {
	t.Parallel()
	secret := "cls_v1.credential.very-secret-value"
	credential, err := NewStaticBearerCredential(secret)
	if err != nil {
		t.Fatal(err)
	}

	for _, rendered := range []string{
		fmt.Sprint(credential),
		fmt.Sprintf("%#v", credential),
		string(mustMarshalJSON(t, credential)),
	} {
		if strings.Contains(rendered, secret) {
			t.Fatal("credential representation disclosed the secret")
		}
	}

	metadata, err := credential.GetRequestMetadata(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if metadata["authorization"] != "Bearer "+secret {
		t.Fatal("credential did not produce expected authorization metadata")
	}
	if !credential.RequireTransportSecurity() {
		t.Fatal("credential must require transport security")
	}
}

func mustMarshalJSON(t *testing.T, value any) []byte {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestCredentialRejectsHeaderInjection(t *testing.T) {
	t.Parallel()
	if _, err := NewStaticBearerCredential("cls_v1.invalid\nheader"); err == nil {
		t.Fatal("expected credential validation failure")
	}
	if _, err := NewStaticBearerCredential(strings.Repeat("x", 762)); err == nil {
		t.Fatal("expected oversized credential validation failure")
	}
}
