// SPDX-License-Identifier: Apache-2.0

package directvolume

import (
	b64 "encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

const (
	MountInfoFileName        = "mountInfo.json"
	KataDirectVolumeRootPath = "/run/kata-containers/shared/direct-volumes"
)

// MountInfo matches kata-containers direct-volume mountInfo.json.
type MountInfo struct {
	VolumeType string            `json:"volume-type"`
	Device     string            `json:"device"`
	FsType     string            `json:"fstype"`
	Metadata   map[string]string `json:"metadata,omitempty"`
	Options    []string          `json:"options,omitempty"`
}

func DirectVolumeDir(targetPath string) string {
	return filepath.Join(KataDirectVolumeRootPath, b64.URLEncoding.EncodeToString([]byte(targetPath)))
}

func WriteMountInfo(targetPath string, info MountInfo) error {
	dir := DirectVolumeDir(targetPath)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	st, err := os.Stat(dir)
	if err != nil {
		return err
	}
	if !st.IsDir() {
		return status.Error(codes.Internal, fmt.Sprintf("%s should be a directory", dir))
	}

	raw, err := json.Marshal(info)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, MountInfoFileName), raw, 0o600)
}

func Remove(targetPath string) error {
	return os.RemoveAll(DirectVolumeDir(targetPath))
}

func MountInfoExists(targetPath string) bool {
	_, err := os.Stat(filepath.Join(DirectVolumeDir(targetPath), MountInfoFileName))
	return err == nil
}

func ReadMountInfo(targetPath string) (*MountInfo, error) {
	raw, err := os.ReadFile(filepath.Join(DirectVolumeDir(targetPath), MountInfoFileName))
	if err != nil {
		return nil, err
	}
	var info MountInfo
	if err := json.Unmarshal(raw, &info); err != nil {
		return nil, err
	}
	return &info, nil
}

func ValidateRootWritable() error {
	if err := os.MkdirAll(KataDirectVolumeRootPath, 0o700); err != nil {
		return err
	}
	test := filepath.Join(KataDirectVolumeRootPath, ".write-test")
	if err := os.WriteFile(test, []byte("ok"), 0o600); err != nil {
		return errors.New("direct-volumes root is not writable")
	}
	_ = os.Remove(test)
	return nil
}
