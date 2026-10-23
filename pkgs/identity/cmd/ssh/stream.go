package ssh

import (
	"context"
	"encoding/json"
	"errors"

	// "strings"
	// "time"

	_ "net/http/pprof"

	"github.com/centrifugal/centrifuge-go"
	"github.com/charmbracelet/log"
	ctx "github.com/developing-today/code/src/identity/cmd/context"
	d "github.com/developing-today/code/src/identity/cmd/do"
	"github.com/developing-today/code/src/identity/cmd/stream"
	"github.com/samber/do/v2"
)

type ChatMessage struct {
	Input string
}

type StreamClientService interface {
	Start()
	Shutdown() error
	HealthCheck() error
	IsStreamClientService() bool
	Subscribe(channel string) (StreamSubscription, error)
}

type StreamClient struct {
	ContextService ctx.ContextService
	Client         *centrifuge.Client
	ctx            context.Context
	cancelFunc     context.CancelFunc
	streamService  stream.StreamService
}

func MustGetStreamClientService(i do.Injector) StreamClientService {
	return d.MustInvokeAny[StreamClientService](i)
}

func NewStreamClientService(i do.Injector) (StreamClientService, error) {
	log.Info("Creating stream client service")
	context := ctx.MustGetContextService(i)
	streamService := stream.MustGetStreamService(i)
	streamClient := StreamClient{
		ContextService: context,
		Client:         NewJsonClient(),
		streamService:  streamService,
	}
	log.Info("Stream client service created, adding default handlers")
	err := streamClient.AddDefaultHandlers()
	if err != nil {
		log.Error("Error adding default handlers", "error", err)
		return nil, err
	}
	log.Info("Stream client service created")
	return &streamClient, nil
}

func (sc *StreamClient) Start() {
	log.Info("Starting stream client service")
	if sc.ctx != nil {
		log.Error("Context is already set, stream service is already running")
		panic("Context is already set, stream service is already running")
	}
	sc.ctx, sc.cancelFunc = context.WithCancel(sc.ContextService.Context())
	err := sc.Connect()
	if err != nil {
		log.Error("Error connecting", "error", err)
		return
	}
}

func (sc *StreamClient) Shutdown() error {
	log.Info("Stream client shutdown requested")
	if sc.cancelFunc != nil && sc.ctx.Err() == nil {
		log.Info("Stream client stopping...")
		sc.cancelFunc()
		if sc.Client == nil {
			log.Error("Client is nil")
			return errors.New("client is nil")
		}
		if sc.Client.State() == centrifuge.StateClosed {
			log.Info("Client already closed")
			return nil
		}
		err := sc.Client.Disconnect()
		if err != nil {
			log.Error("Error disconnecting", "error", err)
			return err
		}
		log.Info("Stream client stopped")
	} else {
		log.Info("Stream client already stopped")
	}
	return nil
}

func (sc *StreamClient) HealthCheck() error {
	// state := sc.Client.State()
	// if state != centrifuge.StateConnected {
	// 	if state == centrifuge.StateDisconnected {
	// 		log.Info("Client is disconnected, reconnecting")
	// 		err := sc.Client.Connect()
	// 		if err != nil {
	// 			return err
	// 		}
	// 	}
	// 	state = sc.Client.State()
	// 	if state == centrifuge.StateConnecting {
	// 		log.Info("Client is connecting, waiting for connection")
	// 		time.Sleep(500 * time.Millisecond)
	// 		state = sc.Client.State()
	// 	}
	// 	if state != centrifuge.StateConnected {
	// 		return errors.New(strings.ToLower("client is not connected, state: " + string(state)))
	// 	}
	// }
	return nil
}

func (sc *StreamClient) IsStreamClientService() bool {
	return true
}

func NewDefaultStreamClient() (client *StreamClient, err error) {
	client = NewStreamClient()
	err = client.AddDefaultHandlers()
	if err != nil {
		log.Error("Error adding default handlers", "error", err)
		return nil, err
	}
	err = client.Connect()
	if err != nil {
		log.Error("Error connecting", "error", err)
		return nil, err
	}
	return client, nil
}

func NewJsonClient() *centrifuge.Client {
	return centrifuge.NewJsonClient(
		"ws://127.0.0.1:8001/connection/websocket",
		centrifuge.Config{},
	)
}

func NewStreamClient() *StreamClient {
	client := NewJsonClient()
	return &StreamClient{
		Client: client,
	}
}

func (sc *StreamClient) AddDefaultHandlers() error {
	client := sc.Client
	if client == nil {
		return errors.New("client is nil")
	}
	client.OnConnecting(func(e centrifuge.ConnectingEvent) {
		log.Info("Connecting", "code", e.Code, "reason", e.Reason)
	})
	client.OnConnected(func(e centrifuge.ConnectedEvent) {
		log.Info("Connected", "clientID", e.ClientID)
	})
	client.OnDisconnected(func(e centrifuge.DisconnectedEvent) {
		log.Info("Disconnected", "code", e.Code, "reason", e.Reason)
	})

	client.OnError(func(e centrifuge.ErrorEvent) {
		log.Error("Error", "error", e.Error)
	})

	client.OnMessage(func(e centrifuge.MessageEvent) {
		log.Info("Message from server", "data", string(e.Data))
	})

	client.OnSubscribed(func(e centrifuge.ServerSubscribedEvent) {
		log.Info("Subscribed to server-side channel", "channel", e.Channel, "wasRecovering", e.WasRecovering, "recovered", e.Recovered)
	})
	client.OnSubscribing(func(e centrifuge.ServerSubscribingEvent) {
		log.Info("Subscribing to server-side channel", "channel", e.Channel)
	})
	client.OnUnsubscribed(func(e centrifuge.ServerUnsubscribedEvent) {
		log.Info("Unsubscribed from server-side channel", "channel", e.Channel)
	})

	client.OnPublication(func(e centrifuge.ServerPublicationEvent) {
		log.Info("Publication from server-side channel", "channel", e.Channel, "data", string(e.Data), "offset", e.Offset)
	})
	client.OnJoin(func(e centrifuge.ServerJoinEvent) {
		log.Info("Join to server-side channel", "channel", e.Channel, "user", e.User, "client", e.Client)
	})
	client.OnLeave(func(e centrifuge.ServerLeaveEvent) {
		log.Info("Leave from server-side channel", "channel", e.Channel, "user", e.User, "client", e.Client)
	})
	return nil
}

func (sc *StreamClient) Connect() error {
	client := sc.Client
	if client == nil {
		return errors.New("client is nil")
	}
	err := client.Connect()
	if err != nil {
		log.Error("Error connecting", "error", err)
		return err
	}
	return nil
}

type StreamSubscription struct {
	Subscription *centrifuge.Subscription
}

func (sc *StreamClient) NewSubscription(channel string, config ...centrifuge.SubscriptionConfig) (sub StreamSubscription, err error) {
	s, ok := sc.Client.GetSubscription(channel)
	if ok {
		log.Info("Subscription already exists", "channel", channel)
		return StreamSubscription{Subscription: s}, nil
	}
	log.Info("Creating subscription", "channel", channel)
	s, err = sc.Client.NewSubscription(channel, config...)
	if err != nil {
		log.Error("Error creating subscription", "error", err)
		return StreamSubscription{}, err
	}
	log.Info("Subscription created", "channel", channel)
	return StreamSubscription{Subscription: s}, nil
}

func (ss *StreamSubscription) AddDefaultHandlers() error {
	sub := ss.Subscription
	if sub == nil {
		return errors.New("subscription is nil")
	}
	sub.OnSubscribing(func(e centrifuge.SubscribingEvent) {
		log.Info("Subscribing on channel", "channel", sub.Channel, "code", e.Code, "reason", e.Reason)
	})
	sub.OnSubscribed(func(e centrifuge.SubscribedEvent) {
		log.Info("Subscribed on channel", "channel", sub.Channel, "wasRecovering", e.WasRecovering, "recovered", e.Recovered)
	})
	sub.OnUnsubscribed(func(e centrifuge.UnsubscribedEvent) {
		log.Info("Unsubscribed from channel", "channel", sub.Channel, "code", e.Code, "reason", e.Reason)
	})
	sub.OnError(func(e centrifuge.SubscriptionErrorEvent) {
		log.Error("Subscription error", "channel", sub.Channel, "error", e.Error)
	})
	sub.OnPublication(func(e centrifuge.PublicationEvent) {
		log.Info("Publication from channel", "channel", sub.Channel, "data", string(e.Data), "offset", e.Offset)
		var chatMessage *ChatMessage
		err := json.Unmarshal(e.Data, &chatMessage)
		if err != nil {
			log.Error("Publication error", "err", err)
			return
		}
		log.Info("Someone says", "channel", sub.Channel, "input", chatMessage.Input, "offset", e.Offset)
	})
	sub.OnJoin(func(e centrifuge.JoinEvent) {
		log.Info("Someone joined", "channel", sub.Channel, "user", e.User, "client", e.Client)
	})
	sub.OnLeave(func(e centrifuge.LeaveEvent) {
		log.Info("Someone left", "channel", sub.Channel, "user", e.User, "client", e.Client)
	})
	return nil
}

func (ss *StreamSubscription) Subscribe() error {
	err := ss.Subscription.Subscribe()
	if err != nil {
		log.Error("Error subscribing", "error", err)
		return err
	}
	return nil
}

func (sc *StreamClient) GetSubscription(channel string) (StreamSubscription, bool) {
	sub, ok := sc.Client.GetSubscription(channel)
	if ok {
		return StreamSubscription{Subscription: sub}, true
	}
	return StreamSubscription{}, false
}

func (sc *StreamClient) Subscribe(channel string) (sub StreamSubscription, err error) {
	sub, ok := sc.GetSubscription(channel)
	if ok {
		return sub, nil
	}
	sub, err = sc.NewSubscription(channel)
	if err != nil {
		log.Error("Error creating subscription", "error", err)
		return sub, err
	}
	err = sub.AddDefaultHandlers()
	if err != nil {
		log.Error("Error adding default handlers", "error", err)
		return sub, err
	}
	err = sub.Subscribe()
	if err != nil {
		log.Error("Error subscribing", "error", err)
		return sub, err
	}
	return sub, nil
}
