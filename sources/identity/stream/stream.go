package stream

import (
	"context"
	"time"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/spf13/cobra"
)

func RunStreamServer(ctx context.Context, config *configuration.IdentityServerConfiguration, cmd *cobra.Command, args []string) {
	log.Info("Starting stream server")
	time.Sleep(5 * time.Second)

	// TODO: Implement NATS server, when this shuts down it crashes the rest of the server. uncomment and fix

	// // Extract TLS configuration from IdentityServerConfiguration
	// certFile := config.Configuration.String("tls.cert.server")
	// keyFile := config.Configuration.String("tls.key.server")
	// caFile := config.Configuration.String("tls.ca")
	// host := config.Configuration.String("nats.host")
	// port := config.Configuration.Int("nats.port")

	// serverTlsConfig, err := server.GenTLSConfig(&server.TLSConfigOpts{
	// 	CertFile: certFile,
	// 	KeyFile:  keyFile,
	// 	CaFile:   caFile,
	// 	Verify:   true,
	// })
	// if err != nil {
	// 	log.Fatalf("TLS config error: %v", err)
	// }

	// // Setup the embedded server options.
	// opts := server.Options{
	// 	Host:      host,
	// 	Port:      port,
	// 	TLSConfig: serverTlsConfig,
	// }

	// // Initialize and start the NATS server
	// ns, err := server.NewServer(&opts)
	// if err != nil {
	// 	log.Fatalf("Server initialization error: %v", err)
	// }

	// go func() {
	// 	log.Info("NATS server starting")
	// 	ns.Start()
	// 	if ns.ReadyForConnections(10 * time.Second) {
	// 		log.Info("NATS server ready for connections")
	// 	}
	// }()
	// defer func() {
	// 	log.Info("NATS server shutting down")
	// 	ns.Shutdown()
	// 	log.Info("Stopping stream server")
	// }()

	// select {
	// case <-ctx.Done():
	// 	log.Info("Stream server cancelled")
	// 	return
	// }
}
