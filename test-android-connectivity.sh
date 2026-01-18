#!/usr/bin/env bash
# Android OTG/MTP Connectivity Test Script for NixOS

echo "🔍 Testing Android OTG/MTP Connectivity on NixOS"
echo "================================================"

# Check if required packages are installed
echo "📦 Checking required packages..."
packages=("adb" "jmtpfs" "go-mtpfs" "simple-mtpfs" "libmtp" "usbutils")
missing_packages=()

for package in "${packages[@]}"; do
  if ! command -v "$package" &>/dev/null; then
    missing_packages+=("$package")
  else
    echo "✅ $package is installed"
  fi
done

if [ ${#missing_packages[@]} -gt 0 ]; then
  echo "❌ Missing packages: ${missing_packages[*]}"
  echo "Please run: nixos-rebuild switch"
  exit 1
fi

# Check user groups
echo ""
echo "👤 Checking user groups..."
if groups $USER | grep -q "adbusers"; then
  echo "✅ User is in adbusers group"
else
  echo "❌ User is not in adbusers group"
  echo "Please add user to adbusers group and relogin"
fi

# Check kernel modules
echo ""
echo "🔧 Checking kernel modules..."
modules=("usb_storage" "usbcore" "usb_common" "uas")
for module in "${modules[@]}"; do
  if lsmod | grep -q "$module"; then
    echo "✅ $module is loaded"
  else
    echo "⚠️  $module is not currently loaded (may load on demand)"
  fi
done

# Check services
echo ""
echo "🛠️  Checking services..."
if systemctl is-active --quiet gvfs-daemon; then
  echo "✅ GVFS daemon is running"
else
  echo "❌ GVFS daemon is not running"
fi

if systemctl is-active --quiet devmon; then
  echo "✅ Devmon service is running"
else
  echo "⚠️  Devmon service is not running (optional)"
fi

# Check USB devices
echo ""
echo "🔌 Checking USB devices..."
echo "Connected USB devices:"
lsusb

# Check for Android devices specifically
echo ""
echo "📱 Checking for Android devices..."
if adb devices | grep -q "device$"; then
  echo "✅ Android device(s) found via ADB:"
  adb devices
else
  echo "❌ No Android devices found via ADB"
  echo "Make sure USB debugging is enabled on your Android device"
fi

# Check for MTP devices
echo ""
echo "📁 Checking for MTP devices..."
if command -v simple-mtpfs &>/dev/null; then
  if simple-mtpfs --list-devices 2>/dev/null | grep -q "Simple MTP"; then
    echo "✅ MTP device(s) found:"
    simple-mtpfs --list-devices
  else
    echo "❌ No MTP devices found"
    echo "Make sure MTP is enabled on your Android device"
  fi
fi

echo ""
echo "🔧 Troubleshooting Tips:"
echo "1. On your Android device, enable 'File Transfer' or 'MTP' mode"
echo "2. Enable 'USB Debugging' in Developer Options"
echo "3. Try different USB cables and ports"
echo "4. Restart the adb service: sudo systemctl restart adb"
echo "5. Replug your Android device after making changes"
echo "6. If using Wayland, some file managers may have limited MTP support"

echo ""
echo "🎯 To mount MTP device manually:"
echo "mkdir -p ~/mtp"
echo "simple-mtpfs --device 1 ~/mtp"
echo "fusermount -u ~/mtp  # to unmount"

echo ""
echo "✨ Test completed!"
