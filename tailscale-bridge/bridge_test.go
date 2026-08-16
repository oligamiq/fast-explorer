package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestStableHostnameUsesConfiguredShortName(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "hostname"), []byte("fe-work\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := stableHostname(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got != "fe-work" {
		t.Fatalf("stableHostname() = %q, want %q", got, "fe-work")
	}
}

func TestStableHostnameShortensLegacyDeviceID(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "device-id"), []byte("12345678\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := stableHostname(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got != "fe-123456" {
		t.Fatalf("stableHostname() = %q, want %q", got, "fe-123456")
	}
}
