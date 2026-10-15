// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

// AllocateBlankImage creates a new all-zero (sparse) image file. The CDF
// layout is written later by the guest CDH, which holds K_root; the host only
// ever hands over a blank disk.
func AllocateBlankImage(imagePath string, sizeBytes int64) error {
	f, err := os.OpenFile(imagePath, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return fmt.Errorf("create %s: %w", imagePath, err)
	}
	if err := f.Truncate(sizeBytes); err != nil {
		f.Close()
		_ = os.Remove(imagePath)
		return fmt.Errorf("truncate %s to %d: %w", imagePath, sizeBytes, err)
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(imagePath)
		return fmt.Errorf("close %s: %w", imagePath, err)
	}
	return nil
}

func LosetupFindShow(imagePath string) (string, error) {
	cmd := exec.Command("losetup", "--find", "--show", "--partscan", imagePath)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return "", fmt.Errorf("losetup: %w: %s", err, strings.TrimSpace(stderr.String()))
	}
	dev := strings.TrimSpace(stdout.String())
	if dev == "" {
		return "", fmt.Errorf("losetup returned empty device for %s", imagePath)
	}
	return dev, nil
}

func LosetupDetach(device string) error {
	cmd := exec.Command("losetup", "-d", device)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("losetup -d %s: %w: %s", device, err, strings.TrimSpace(stderr.String()))
	}
	return nil
}
