//go:build android

package main

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"

	"tailscale.com/logpolicy"
)

var (
	platformRuntimeOnce sync.Once
	platformRuntimeErr  error
	platformRuntimeBase string
)

func preparePlatformRuntime(stateDir string) error {
	base := filepath.Join(filepath.Dir(stateDir), "_runtime")
	platformRuntimeOnce.Do(func() {
		platformRuntimeBase = base
		logsDir := filepath.Join(base, "logs")
		if err := os.MkdirAll(logsDir, 0o700); err != nil {
			platformRuntimeErr = fmt.Errorf("create Android Tailscale log directory: %w", err)
			return
		}
		logpolicy.SetFastExplorerAndroidLogsDir(logsDir)
	})
	if platformRuntimeErr != nil {
		return platformRuntimeErr
	}
	if platformRuntimeBase != base {
		return fmt.Errorf(
			"Android Tailscale runtime root changed from %q to %q",
			platformRuntimeBase,
			base,
		)
	}
	return nil
}
