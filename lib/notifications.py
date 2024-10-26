# # import glib
# # import dbus
# # from dbus.mainloop.glib import DBusGMainLoop

# # def notifications(bus, message):
# #     if message.get_member() == "Notify":
# #         print [arg for arg in message.get_args_list()]

# # DBusGMainLoop(set_as_default=True)

# # bus = dbus.SessionBus()
# # bus.add_match_string_non_blocking("interface='org.freedesktop.Notifications'")
# # bus.add_message_filter(notifications)

# # mainloop = glib.MainLoop()
# # mainloop.run()
# import glib
# import dbus
# from dbus.mainloop.glib import DBusGMainLoop

# def print_notification(bus, message):
#   keys = ["app_name", "replaces_id", "app_icon", "summary",
#           "body", "actions", "hints", "expire_timeout"]
#   args = message.get_args_list()
#   if len(args) == 8:
#     notification = dict([(keys[i], args[i]) for i in range(8)])
#     print notification["summary"], notification["body"]

# loop = DBusGMainLoop(set_as_default=True)
# session_bus = dbus.SessionBus()
# session_bus.add_match_string("type='method_call',interface='org.freedesktop.Notifications',member='Notify',eavesdrop=true")
# session_bus.add_message_filter(print_notification)

# glib.MainLoop().run()
"""
http://bazaar.launchpad.net/~jconti/recent-notifications/trunk/view/head:/recent_notifications/Notification.py
Notification.py
by Jason Conti
February 19, 2010

Monitors DBUS for org.freedesktop.Notifications.Notify messages, parses them,
and notifies listeners when they arrive.
"""

import dbus
import glib
import gobject
import gtk
import logging

from dbus.mainloop.glib import DBusGMainLoop
from locale import nl_langinfo, T_FMT

import Timestamp

from Icon import load_icon, load_icon_from_file

logger = logging.getLogger("Notification")

class ImageDataException(Exception):
  pass

class ImageData(object):
  """Parses the image_data hint from a DBUS message."""
  def __init__(self, image_data):
    if len(image_data) < 7:
      raise ImageDataException("Invalid image_data: " + repr(image_data))

    self.width = int(image_data[0])
    self.height = int(image_data[1])
    self.rowstride = int(image_data[2])
    self.has_alpha = bool(image_data[3])
    self.bits_per_sample = int(image_data[4])
    self.channels = int(image_data[5])
    self.data = self.dbus_array_to_str(image_data[6])

  def dbus_array_to_str(self, array):
    return "".join(map(chr, array))

  def get_pixbuf(self, size):
    """Creates a pixbuf from the image data and scale it to the appropriate size."""
    pixbuf = gtk.gdk.pixbuf_new_from_data(self.data, gtk.gdk.COLORSPACE_RGB,
        self.has_alpha, self.bits_per_sample, self.width, self.height,
        self.rowstride)

    return pixbuf.scale_simple(size, size, gtk.gdk.INTERP_BILINEAR)

class MessageException(Exception):
  pass

class Message(gobject.GObject):
  """Parses a DBUS message in the Notify format specified at:
  http://www.galago-project.org/specs/notification/0.9/index.html"""
  # Message urgency
  LOW = 0
  NORMAL = 1
  CRITICAL = 2
  X_CANONICAL_PRIVATE_SYNCHRONOUS = "x-canonical-private-synchronous"
  def __init__(self, dbus_message = None, timestamp = None):
    gobject.GObject.__init__(self)

    self.timestamp = timestamp

    args = dbus_message.get_args_list()

    if len(args) != 8:
      raise MessageException("Invalid message args_list: " + repr(args))

    self.app_name = str(args[0])
    self.replaces_id = args[1]
    self.app_icon = str(args[2])
    self.summary = str(args[3])
    self.body = str(args[4])
    self.actions = args[5]
    self.hints = dict(args[6])
    self.expire_timeout = args[7]

    if "urgency" in self.hints:
      urgency = self.hints["urgency"]
      if urgency == 0:
        self.urgency = Message.LOW
      elif urgency == 2:
        self.urgency = Message.CRITICAL
      else:
        self.urgency = Message.NORMAL
    else:
      self.urgency = Message.NORMAL

    if "image_data" in self.hints:
      self.image_data = ImageData(self.hints["image_data"])
    else:
      self.image_data = None

    if "icon_data" in self.hints:
      self.icon_data = ImageData(self.hints["icon_data"])
    else:
      self.icon_data = None

    if "image-path" in self.hints:
      self.image_path = self.hints["image-path"]
    else:
      self.image_path = None

    self.log_message()

  def get_icon(self, size = 48):
    """Loads the icon into a pixbuf."""
    if self.image_path != None:
      icon_name = self.image_path
    else:
      icon_name = self.app_icon

    # Try to load the image data from the message
    if self.image_data != None:
      return self.image_data.get_pixbuf(size)

    # Try to load the pixbuf from a file
    elif icon_name.startswith("file://") or icon_name.startswith("/"):
      icon = load_icon_from_file(icon_name, size)
      if icon != None:
        return icon

    # Try to load the pixbuf from the current icon theme
    elif icon_name != "":
      icon = load_icon(icon_name, size)
      if icon != None:
        return icon

    # Try to load the icon data from the message
    elif self.icon_data != None:
      return self.icon_data.get_pixbuf(size)

    return self.get_default_icon(size)

  def get_default_icon(self, size = 48):
    """Attempts to load the default message icon, returns None on failure."""
    if self.urgency == Message.LOW:
      return load_icon("notification-low", size)
    elif self.urgency == Message.CRITICAL:
      return load_icon("notification-critical", size)
    else:
      return load_icon("notification-normal", size)

  def is_volume_notification(self):
    """Returns true if this is a volume message. The volume notifications
    in Ubuntu are a special case, and mostly a blank message. Clutters up
    the display and provides no useful information, so it is better to
    discard them."""
    if Message.X_CANONICAL_PRIVATE_SYNCHRONOUS in self.hints:
      return str(self.hints[Message.X_CANONICAL_PRIVATE_SYNCHRONOUS]) == "volume"

  def log_message(self):
    """Write debug info about a message."""
    result = [
        "-" * 50,
        "Message created at: " + Timestamp.locale_datetime(self.timestamp),
        "app_name: " + repr(self.app_name),
        "replaces_id: " + repr(self.replaces_id),
        "app_icon: " + repr(self.app_icon),
        "summary: " + repr(self.summary),
        "body: " + repr(self.body),
        "actions: " + repr(self.actions),
        "expire_timeout: " + repr(self.expire_timeout),
        "hints:"
        ]

    # Log all the hints except image_data
    for key in self.hints:
      if key != "image_data":
        result.append("  " + str(key) + ": " + repr(self.hints[key]))

    # Log info about the image_data
    if self.image_data != None:
      result.append("image_data:")
      result.append("  width: " + repr(self.image_data.width))
      result.append("  height: " + repr(self.image_data.height))
      result.append("  rowstride: " + repr(self.image_data.rowstride))
      result.append("  has_alpha: " + repr(self.image_data.has_alpha))
      result.append("  bits_per_sample: " + repr(self.image_data.bits_per_sample))
      result.append("  channels: " + repr(self.image_data.channels))

    result.append("-" * 50)

    logger.debug("\n" + "\n".join(result))

class Notification(gobject.GObject):
  """Monitors DBUS for org.freedesktop.Notifications.Notify messages, parses them,
  and notifies listeners when they arrive."""
  __gsignals__ = {
      "message-received": (gobject.SIGNAL_RUN_LAST, gobject.TYPE_NONE, [Message])
  }
  def __init__(self):
    gobject.GObject.__init__(self)

    self._blacklist = None
    self._match_string = "type='method_call',interface='org.freedesktop.Notifications',member='Notify'"

    DBusGMainLoop(set_as_default=True)
    self._bus = dbus.SessionBus()
    self._bus.add_match_string(self._match_string)
    self._bus.add_message_filter(self._message_filter)

  def close(self):
    """Closes the connection to the session bus."""
    self._bus.close()

  def set_blacklist(self, blacklist):
    """Defines the set of app_names of messages to be discarded."""
    self._blacklist = blacklist

  def _message_filter(self, connection, dbus_message):
    """Triggers when messages are received from the session bus."""
    if dbus_message.get_member() == "Notify" and dbus_message.get_interface() == "org.freedesktop.Notifications":
      try:
        message = Message(dbus_message, Timestamp.now())
      except:
        logger.exception("Failed to parse dbus message: " + repr(dbus_message.get_args_list()))
      else:
        # Discard unwanted messages
        if message.is_volume_notification():
          return
        if self._blacklist and self._blacklist.get_bool(message.app_name, False):
          return
        glib.idle_add(self.emit, "message-received", message)

if gtk.pygtk_version < (2, 8, 0):
  gobject.type_register(Message)
  gobject.type_register(Notification)

if __name__ == '__main__':
  main()
