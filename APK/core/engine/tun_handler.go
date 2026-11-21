package engine

import (
	"context"
	"net"
	"net/netip"

	"github.com/metacubex/mihomo/component/resolver"
	"github.com/metacubex/mihomo/listener/sing_tun"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/sing/common/buf"
	M "github.com/metacubex/sing/common/metadata"
	N "github.com/metacubex/sing/common/network"
)

type sessionHandler struct {
	*sing_tun.ListenerHandler
	tasks *sessionTasks
}

func (h *sessionHandler) NewConnection(_ context.Context, connection net.Conn, metadata M.Metadata) error {
	if tunnel.Status() != tunnel.Running {
		return connection.Close()
	}
	if !h.tasks.begin(connection) {
		return connection.Close()
	}
	defer h.tasks.end(connection)
	return h.ListenerHandler.NewConnection(h.tasks.ctx, connection, metadata)
}

func (h *sessionHandler) NewPacket(_ context.Context, _ netip.AddrPort, input *buf.Buffer, metadata M.Metadata, init func(N.PacketConn) N.PacketWriter) {
	if tunnel.Status() != tunnel.Running || metadata.Destination.Port != 53 || !h.tasks.begin(nil) {
		input.Release()
		return
	}
	writer := init(nil)
	go func() {
		defer h.tasks.end(nil)
		defer input.Release()
		ctx, cancel := context.WithTimeout(h.tasks.ctx, resolver.DefaultDnsRelayTimeout)
		defer cancel()
		options := N.ReadWaitOptions{
			FrontHeadroom: N.CalculateFrontHeadroom(writer), RearHeadroom: N.CalculateRearHeadroom(writer),
			MTU: resolver.SafeDnsPacketSize,
		}
		output := options.NewPacketBuffer()
		response, err := resolver.RelayDnsPacket(ctx, input.Bytes(), output.FreeBytes())
		if err != nil {
			output.Release()
			return
		}
		output.Truncate(len(response))
		if err = writer.WritePacket(output, metadata.Destination); err != nil {
			output.Release()
		}
	}()
}
