#!/usr/bin/env python3
import dbus
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
import argparse
import json
import os
import sys

class NotificationTracker:
    def __init__(self):
        self.notifications = {}
        self.setup_dbus()

    def setup_dbus(self):
        DBusGMainLoop(set_as_default=True)
        self.bus = dbus.SessionBus()
        
        # Monitor for new notifications
        self.bus.add_match_string(
            "type='method_call',interface='org.freedesktop.Notifications',member='Notify'"
        )
        self.bus.add_message_filter(self.notification_handler)

    def notification_handler(self, bus, message):
        try:
            args = message.get_args_list()
            if len(args) >= 1:
                notification_id = args[0]
                sender = message.get_sender()
                
                # Get PID from D-Bus
                proxy = self.bus.get_object('org.freedesktop.DBus', '/org/freedesktop/DBus')
                dbus_interface = dbus.Interface(proxy, 'org.freedesktop.DBus')
                pid = dbus_interface.GetConnectionUnixProcessID(sender)
                
                self.notifications[str(notification_id)] = {
                    'pid': pid,
                    'sender': sender,
                    'app_name': args[1] if len(args) > 1 else 'Unknown'
                }
                
                # Save to cache file
                self.save_cache()
                
        except Exception as e:
            print(f"Error handling notification: {e}", file=sys.stderr)

    def save_cache(self):
        cache_dir = os.path.expanduser('~/.cache/notification-tracker')
        os.makedirs(cache_dir, exist_ok=True)
        cache_file = os.path.join(cache_dir, 'notifications.json')
        
        try:
            with open(cache_file, 'w') as f:
                json.dump(self.notifications, f)
        except Exception as e:
            print(f"Error saving cache: {e}", file=sys.stderr)

    def load_cache(self):
        cache_file = os.path.expanduser('~/.cache/notification-tracker/notifications.json')
        try:
            if os.path.exists(cache_file):
                with open(cache_file, 'r') as f:
                    self.notifications = json.load(f)
        except Exception as e:
            print(f"Error loading cache: {e}", file=sys.stderr)

    def get_pid(self, notification_id):
        notification_id = str(notification_id)
        if notification_id in self.notifications:
            return self.notifications[notification_id]['pid']
        return None

def main():
    parser = argparse.ArgumentParser(description='Get PID for a notification ID')
    parser.add_argument('notification_id', type=str, help='Notification ID to look up')
    parser.add_argument('--monitor', action='store_true', help='Monitor for new notifications')
    args = parser.parse_args()

    tracker = NotificationTracker()
    tracker.load_cache()

    if args.monitor:
        print("Monitoring for notifications... Press Ctrl+C to stop")
        loop = GLib.MainLoop()
        try:
            loop.run()
        except KeyboardInterrupt:
            print("\nStopping monitor...")
            return

    pid = tracker.get_pid(args.notification_id)
    if pid:
        print(f"PID: {pid}")
    else:
        print(f"No PID found for notification ID: {args.notification_id}")

if __name__ == '__main__':
    main()