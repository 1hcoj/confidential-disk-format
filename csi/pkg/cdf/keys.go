// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	ParamKbsKeyID          = "cdf.kbs-key-id"
	ParamProvisionKeyFile  = "cdf.provision-key-file"
	AnnotationKbsKeyID     = "cdf.confidentialcontainers.org/kbs-key-id"
	EnvProvisionKeyDir     = "CDF_PROVISION_KEY_DIR"
)

// ResolveProvisionKeyFile returns a host path to K_root material for sealing.
func ResolveProvisionKeyFile(params map[string]string) (string, error) {
	if p := strings.TrimSpace(params[ParamProvisionKeyFile]); p != "" {
		return p, nil
	}

	kbsID := ResolveKbsKeyID(params)
	if kbsID == "" {
		return "", fmt.Errorf("missing key locator: set %q, %q, or %q", AnnotationKbsKeyID, ParamKbsKeyID, ParamProvisionKeyFile)
	}

	base := os.Getenv(EnvProvisionKeyDir)
	if base == "" {
		return "", fmt.Errorf("%s is unset and %q not provided", EnvProvisionKeyDir, ParamProvisionKeyFile)
	}

	rel, err := kbsURIToRelativePath(kbsID)
	if err != nil {
		return "", err
	}
	path := filepath.Join(base, rel)
	if _, err := os.Stat(path); err != nil {
		return "", fmt.Errorf("provision key file %q: %w", path, err)
	}
	return path, nil
}

func ResolveKbsKeyID(params map[string]string) string {
	for _, k := range []string{AnnotationKbsKeyID, ParamKbsKeyID} {
		if v := strings.TrimSpace(params[k]); v != "" {
			return v
		}
	}
	return ""
}

func kbsURIToRelativePath(uri string) (string, error) {
	u := strings.TrimSpace(uri)
	switch {
	case strings.HasPrefix(u, "kbs:///"):
		return strings.TrimPrefix(u, "kbs:///"), nil
	case strings.HasPrefix(u, "kbs://"):
		rest := strings.TrimPrefix(u, "kbs://")
		rest = strings.TrimPrefix(rest, "/")
		if rest == "" {
			return "", fmt.Errorf("invalid kbs URI: %q", uri)
		}
		return rest, nil
	default:
		return "", fmt.Errorf("expected kbs URI, got %q", uri)
	}
}
