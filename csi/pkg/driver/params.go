// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"fmt"
	"os"
	"strings"

	cdfpkg "github.com/confidential-containers/confidential-disk-format/csi/pkg/cdf"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func volumeDirFrom(params map[string]string, defaultDir string) string {
	if v := strings.TrimSpace(params[ParamVolumeDir]); v != "" {
		return v
	}
	if v := strings.TrimSpace(os.Getenv("CDF_VOLUME_DIR")); v != "" {
		return v
	}
	if defaultDir != "" {
		return defaultDir
	}
	return DefaultVolumeDir
}

func requireBareMetal(params map[string]string) error {
	platform := strings.TrimSpace(params[ParamPlatform])
	if platform == "" {
		return status.Error(codes.InvalidArgument, ParamPlatform+" is required")
	}
	if platform != PlatformBareMetal {
		return status.Errorf(codes.Unimplemented, "MVP supports %q only, got %q", PlatformBareMetal, platform)
	}
	return nil
}

func mergeVolumeContext(base map[string]string) map[string]string {
	out := make(map[string]string, len(base))
	for k, v := range base {
		out[k] = v
	}
	return out
}

func publishMetadata(ctx map[string]string) map[string]string {
	keys := []string{
		ParamPlatform,
		ParamProfile,
		cdfpkg.ParamKbsKeyID,
		ParamExpectedVolumeUUID,
		ParamMkfsIfEmpty,
	}
	meta := make(map[string]string)
	for _, k := range keys {
		if v := strings.TrimSpace(ctx[k]); v != "" {
			meta[k] = v
		}
	}
	// Alias for CDH: metadata uses cdf.kbs-key-id
	if _, ok := meta["cdf.kbs-key-id"]; !ok {
		if v := strings.TrimSpace(ctx["cdf.confidentialcontainers.org/kbs-key-id"]); v != "" {
			meta["cdf.kbs-key-id"] = v
		}
	}
	return meta
}

func imagePathFromContext(ctx map[string]string) (string, error) {
	if hp := strings.TrimSpace(ctx[ParamHostPath]); hp != "" {
		return hp, nil
	}
	return "", fmt.Errorf("volume context missing %s", ParamHostPath)
}

func fstypeFrom(ctx map[string]string) string {
	if v := strings.TrimSpace(ctx[ParamFstype]); v != "" {
		return v
	}
	return DefaultFstype
}

func profileFrom(ctx map[string]string) string {
	if v := strings.TrimSpace(ctx[ParamProfile]); v != "" {
		return v
	}
	return DefaultProfile
}
