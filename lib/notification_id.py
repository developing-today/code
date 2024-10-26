#!/usr/bin/env python3
import dbus
import gi
#gi.require_version('Gtk', '3.0')
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
from gi.repository import GObject

def notification_callback(bus_name, object_path, interface, signal_name, parameters, *args):
    notification_id = parameters[0]  # First parameter is the notification ID
    app_name = parameters[1]         # Second parameter is the app name
    summary = parameters[3]          # Fourth parameter is the summary/title
    body = parameters[4]             # Fifth parameter is the body

    print(f"Notification ID: {notification_id}")
    print(f"Application: {app_name}")
    print(f"Summary: {summary}")
    print(f"Body: {body}")
    print("---")

def main():
    # Initialize D-Bus main loop
    DBusGMainLoop(set_as_default=True)

    # Connect to the session bus
    bus = dbus.SessionBus()

    # Subscribe to notifications
    bus.add_match_string(
        "type='method_call',interface='org.freedesktop.Notifications',member='Notify'"
    )
    bus.add_message_filter(notification_callback)

    # Start the main loop
    loop = GLib.MainLoop()
    print("Monitoring notifications... (Press Ctrl+C to exit)")
    try:
        loop.run()
    except KeyboardInterrupt:
        print("\nStopping notification monitor")

if __name__ == "__main__":
    main()
