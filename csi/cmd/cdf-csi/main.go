// SPDX-License-Identifier: Apache-2.0

package main

import (
	"flag"
	"os"

	"github.com/confidential-containers/confidential-disk-format/csi/pkg/driver"
	"k8s.io/klog/v2"
)

var version = "dev"

func main() {
	klog.InitFlags(nil)
	cfg := driver.Config{
		VendorVersion: version,
	}

	flag.StringVar(&cfg.Endpoint, "endpoint", "unix:///var/lib/kubelet/plugins/cdf.csi.confidentialcontainers.org/csi.sock", "CSI gRPC endpoint")
	flag.StringVar(&cfg.NodeID, "nodeid", "", "Kubernetes node name")
	flag.StringVar(&cfg.StateDir, "statedir", "/var/lib/cdf-csi/state", "Persistent state directory")
	flag.StringVar(&cfg.DefaultVolDir, "volume-dir", driver.DefaultVolumeDir, "Default directory for dynamically provisioned CDF images")
	showVersion := flag.Bool("version", false, "Print version and exit")
	flag.Parse()

	if *showVersion {
		klog.Infof("cdf-csi %s", version)
		return
	}
	if cfg.NodeID == "" {
		klog.Error("--nodeid is required")
		os.Exit(1)
	}

	d, err := driver.New(cfg)
	if err != nil {
		klog.Fatalf("driver init: %v", err)
	}
	if err := d.Run(); err != nil {
		klog.Fatalf("driver run: %v", err)
	}
}
