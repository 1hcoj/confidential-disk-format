// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveKbsKeyIDPrefersAnnotation(t *testing.T) {
	got := ResolveKbsKeyID(map[string]string{
		ParamKbsKeyID:      "kbs:///default/cdf/sc",
		AnnotationKbsKeyID: " kbs:///default/cdf/pvc ",
	})
	if got != "kbs:///default/cdf/pvc" {
		t.Fatalf("got %q", got)
	}
}

func TestAllocateBlankImage(t *testing.T) {
	p := filepath.Join(t.TempDir(), "v.img")
	if err := AllocateBlankImage(p, 64<<20); err != nil {
		t.Fatal(err)
	}
	st, err := os.Stat(p)
	if err != nil || st.Size() != 64<<20 {
		t.Fatalf("stat %v size=%d", err, st.Size())
	}
	if err := AllocateBlankImage(p, 64<<20); err == nil {
		t.Fatal("expected refusal to reuse an existing image")
	}
}
