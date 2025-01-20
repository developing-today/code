date
yum install -y nvme-cli smartmontools
lsblk
lsblk -o name,size,fstype,label,model,serial,mountpoint
ls -la /dev/disk/by-uuid
ls -la /dev/disk/by-id
devices=$(lsblk -d -o NAME -n | grep -E '^(sd|nvme|hd)')

for dev in $devices; do
    echo "=== Device: /dev/$dev ==="
    smartctl -x "/dev/$dev"

    if [[ $dev == nvme* ]]; then
        echo "=== NVMe Specific Info ==="
        # nvme id-ctrl "/dev/$dev"
        nvme smart-log "/dev/$dev"
        # nvme error-log "/dev/$dev"
    fi
    echo ""
done
date

#
