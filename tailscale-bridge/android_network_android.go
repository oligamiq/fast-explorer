//go:build android

package main

import (
	"encoding/json"
	"net"
	"net/netip"
	"sync"

	"tailscale.com/net/netmon"
)

type androidInterfaceSnapshot struct {
	Name      string   `json:"name"`
	MTU       int      `json:"mtu"`
	Addresses []string `json:"addresses"`
}

var androidNetworkState struct {
	mu         sync.RWMutex
	interfaces []netmon.Interface
}

func init() {
	netmon.RegisterInterfaceGetter(func() ([]netmon.Interface, error) {
		androidNetworkState.mu.RLock()
		defer androidNetworkState.mu.RUnlock()
		return append([]netmon.Interface(nil), androidNetworkState.interfaces...), nil
	})
}

func setAndroidInterfacesJSON(raw string) error {
	var snapshots []androidInterfaceSnapshot
	if err := json.Unmarshal([]byte(raw), &snapshots); err != nil {
		return err
	}
	interfaces := make([]netmon.Interface, 0, len(snapshots))
	defaultName := ""
	for index, snapshot := range snapshots {
		if snapshot.Name == "" {
			continue
		}
		if defaultName == "" {
			defaultName = snapshot.Name
		}
		addresses := make([]net.Addr, 0, len(snapshot.Addresses))
		for _, value := range snapshot.Addresses {
			prefix, err := netip.ParsePrefix(value)
			if err != nil {
				continue
			}
			addr := prefix.Addr()
			bits := 128
			if addr.Is4() {
				bits = 32
			}
			addresses = append(addresses, &net.IPNet{
				IP:   net.IP(addr.AsSlice()),
				Mask: net.CIDRMask(prefix.Bits(), bits),
			})
		}
		mtu := snapshot.MTU
		if mtu <= 0 {
			mtu = 1500
		}
		interfaces = append(interfaces, netmon.Interface{
			Interface: &net.Interface{
				Index: index + 1,
				MTU:   mtu,
				Name:  snapshot.Name,
				Flags: net.FlagUp | net.FlagBroadcast | net.FlagMulticast,
			},
			AltAddrs: addresses,
		})
	}

	androidNetworkState.mu.Lock()
	androidNetworkState.interfaces = interfaces
	androidNetworkState.mu.Unlock()
	// An empty interface name explicitly tells Tailscale that the active network was lost.
	netmon.UpdateLastKnownDefaultRouteInterface(defaultName)
	return nil
}
