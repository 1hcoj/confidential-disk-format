// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
)

type volumeRecord struct {
	VolumeID  string            `json:"volumeId"`
	Name      string            `json:"name"`
	HostPath  string            `json:"hostPath"`
	SizeBytes int64             `json:"sizeBytes"`
	Context   map[string]string `json:"context,omitempty"`
}

type volumeState struct {
	dir string
	mu  sync.Mutex
}

func newVolumeState(dir string) (*volumeState, error) {
	if err := os.MkdirAll(dir, 0o750); err != nil {
		return nil, err
	}
	return &volumeState{dir: dir}, nil
}

func (s *volumeState) path(id string) string {
	return filepath.Join(s.dir, id+".json")
}

func (s *volumeState) GetByID(id string) (*volumeRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	raw, err := os.ReadFile(s.path(id))
	if err != nil {
		return nil, err
	}
	var rec volumeRecord
	if err := json.Unmarshal(raw, &rec); err != nil {
		return nil, err
	}
	return &rec, nil
}

func (s *volumeState) GetByName(name string) (*volumeRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	entries, err := os.ReadDir(s.dir)
	if err != nil {
		return nil, err
	}
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		raw, err := os.ReadFile(filepath.Join(s.dir, e.Name()))
		if err != nil {
			continue
		}
		var rec volumeRecord
		if err := json.Unmarshal(raw, &rec); err != nil {
			continue
		}
		if rec.Name == name {
			return &rec, nil
		}
	}
	return nil, os.ErrNotExist
}

func (s *volumeState) Put(rec *volumeRecord) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	raw, err := json.Marshal(rec)
	if err != nil {
		return err
	}
	return os.WriteFile(s.path(rec.VolumeID), raw, 0o600)
}

func (s *volumeState) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return os.Remove(s.path(id))
}
