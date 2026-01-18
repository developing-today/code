# Android OTG/MTP Connectivity on NixOS

This configuration provides comprehensive Android OTG/MTP connectivity support for NixOS desktop systems.

## What's Included

### Core Components
- **ADB (Android Debug Bridge)**: For debugging and file transfer
- **MTP Support**: Multiple MTP filesystem implementations
- **USB Device Management**: Proper udev rules and permissions
- **GVFS**: Virtual filesystem for seamless integration

### Packages Installed
- `android-tools` (adb, fastboot)
- `android-udev-rules` (USB device rules)
- `jmtpfs`, `go-mtpfs`, `simple-mtpfs` (MTP filesystems)
- `libmtp` (MTP library)
- `usbutils` (USB utilities)

### Services Enabled
- `adb` (Android Debug Bridge)
- `gvfs` (GNOME Virtual File System)
- `devmon` (Device automounter)

## Usage

### 1. Connect Your Android Device
1. On your Android device, enable "File Transfer" or "MTP" mode
2. For advanced features, enable "USB Debugging" in Developer Options
3. Connect via USB cable

### 2. Access Your Device
**File Managers:**
- GNOME Files: Should automatically detect and show your device
- KDE Dolphin: Should automatically detect and show your device
- Thunar: Should automatically detect and show your device

**Command Line:**
```bash
# List connected devices
adb devices

# List MTP devices
simple-mtpfs --list-devices

# Mount manually (if needed)
mkdir -p ~/mtp
simple-mtpfs --device 1 ~/mtp

# Unmount
fusermount -u ~/mtp
```

### 3. Test Your Setup
Run the provided test script:
```bash
./test-android-connectivity.sh
```

## Troubleshooting

### Device Not Detected
1. **Check USB Mode**: Ensure your Android device is in "File Transfer" or "MTP" mode
2. **Try Different Cable/Port**: Some cables/ports don't support data transfer
3. **Restart Services**: `sudo systemctl restart adb`
4. **Replug Device**: Unplug and replug your Android device

### Permission Issues
1. **Check User Group**: Ensure your user is in the `adbusers` group
2. **Re-login**: You may need to relogin after group changes
3. **Check udev Rules**: Ensure `android-udev-rules` is installed

### MTP Issues
1. **File Manager Support**: Some Wayland file managers have limited MTP support
2. **Manual Mount**: Try mounting manually with `simple-mtpfs`
3. **Alternative Tools**: Try `jmtpfs` or `go-mtpfs` if `simple-mtpfs` doesn't work

### ADB Issues
1. **Enable Debugging**: Make sure "USB Debugging" is enabled in Developer Options
2. **Authorize Device**: Accept the authorization prompt on your Android device
3. **Restart ADB Server**: `sudo adb kill-server && sudo adb start-server`

## Configuration Details

### User Groups
Your user is automatically added to the `adbusers` group for USB device access.

### Udev Rules
Comprehensive udev rules are included for:
- Common Android device vendors
- MTP device detection
- Proper USB permissions

### Kernel Modules
Required kernel modules are loaded:
- `usb_storage` (USB mass storage)
- `usbcore` (USB core)
- `usb_common` (USB common)
- `uas` (USB Attached SCSI)

## File Locations
- Configuration: `nixos/android-connectivity/default.nix`
- Test Script: `test-android-connectivity.sh`
- Hardware Config: `nixos/hardware-configuration/framework/*/default.nix`

## Support
This configuration supports both Framework laptop models:
- Framework 13-inch 12th Gen Intel
- Framework 13-inch 7040 AMD

The configuration should work with most Android devices from major manufacturers (Google, Samsung, Motorola, OnePlus, etc.).