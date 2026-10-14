// SPDX-License-Identifier: Apache-2.0

package cdf

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"strings"

	"k8s.io/apimachinery/pkg/api/resource"
)

const EnvGptBin = "CDF_GPT_BIN"

func GptBin() string {
	if v := strings.TrimSpace(os.Getenv(EnvGptBin)); v != "" {
		return v
	}
	return "cdf-gpt"
}

func CreateVolumeImage(bin, imagePath, profile, keyFile, keyLocator string, sizeBytes int64) error {
	sizeQty := resource.NewQuantity(sizeBytes, resource.BinarySI)
	sizeArg := sizeQty.String()

	args := []string{
		imagePath,
		"create",
		"--profile", profile,
		"--size", sizeArg,
		"--key-file", keyFile,
	}
	if keyLocator != "" {
		args = append(args, "--key-locator", keyLocator)
	}

	cmd := exec.Command(bin, args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("cdf-gpt create: %w: %s", err, strings.TrimSpace(stderr.String()))
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
