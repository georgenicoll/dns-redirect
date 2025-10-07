#!/bin/bash
set -e

# Create dns-redirect user and group
sudo useradd --system --no-create-home --shell /bin/false dns-redirect || true

# Create configuration directory
sudo mkdir -p /etc/dns-redirect

# Copy configuration file if it doesn't exist
if [ ! -f /etc/dns-redirect/config.json ]; then
    sudo cp example-config.json /etc/dns-redirect/config.json
    sudo chown dns-redirect:dns-redirect /etc/dns-redirect/config.json
    sudo chmod 644 /etc/dns-redirect/config.json
fi

# Install binary
sudo cp dns-redirect /usr/local/bin/
sudo chown root:root /usr/local/bin/dns-redirect
sudo chmod 755 /usr/local/bin/dns-redirect

# Install systemd service
sudo cp dns-redirect.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable dns-redirect

echo "Installation complete. To start the service:"
echo "  sudo systemctl start dns-redirect"
echo ""
echo "To check status:"
echo "  sudo systemctl status dns-redirect"
echo ""
echo "Configuration file: /etc/dns-redirect/config.json"