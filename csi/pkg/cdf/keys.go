// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"strings"
)

const (
	ParamKbsKeyID      = "cdf.kbs-key-id"
	AnnotationKbsKeyID = "cdf.confidentialcontainers.org/kbs-key-id"
)

// ResolveKbsKeyID returns the (non-secret) key locator the guest uses to
// fetch K_root. The host never resolves it to key material.
func ResolveKbsKeyID(params map[string]string) string {
	for _, k := range []string{AnnotationKbsKeyID, ParamKbsKeyID} {
		if v := strings.TrimSpace(params[k]); v != "" {
			return v
		}
	}
	return ""
}
