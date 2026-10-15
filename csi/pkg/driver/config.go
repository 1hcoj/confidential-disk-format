// SPDX-License-Identifier: Apache-2.0

package driver

const (
	DriverName = "cdf.csi.confidentialcontainers.org"

	TopologyKeyNode = "topology.cdf.csi/node"

	ParamPlatform           = "cdf.platform"
	ParamProfile            = "cdf.profile"
	ParamFstype             = "cdf.fstype"
	ParamVolumeDir          = "cdf.volume-dir"
	ParamHostPath           = "cdf.host-path"
	ParamBackend            = "cdf.backend"
	ParamExpectedVolumeUUID = "cdf.expected-volume-uuid"

	PlatformBareMetal = "bare-metal"
	BackendFile       = "file"

	DefaultVolumeDir = "/var/lib/cdf-csi/volumes"
	DefaultFstype    = "ext4"
	DefaultProfile   = "crypt+integrity"
)

type Config struct {
	Endpoint      string
	NodeID        string
	VendorVersion string
	StateDir      string
	DefaultVolDir string
}
