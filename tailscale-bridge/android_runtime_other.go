//go:build !android

package main

func preparePlatformRuntime(_ string) error {
	return nil
}
