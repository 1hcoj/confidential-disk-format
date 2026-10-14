// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"os"
	"path/filepath"
	"testing"
)

func TestKbsURIToRelativePath(t *testing.T) {
	p, err := kbsURIToRelativePath("kbs:///default/cdf/my-pvc-key")
	if err != nil || p != "default/cdf/my-pvc-key" {
		t.Fatalf("got %q err=%v", p, err)
	}
}

func TestResolveProvisionKeyFile(t *testing.T) {
	dir := t.TempDir()
	keyPath := filepath.Join(dir, "default", "cdf", "k")
	if err := os.MkdirAll(filepath.Dir(keyPath), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(keyPath, []byte("secret"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv(EnvProvisionKeyDir, dir)

	got, err := ResolveProvisionKeyFile(map[string]string{
		ParamKbsKeyID: "kbs:///default/cdf/k",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got != keyPath {
		t.Fatalf("want %s got %s", keyPath, got)
	}
}
