---
name: add-domains
description: Use when adding new domains to tools.haiocloud.com/domains.txt. Handles SSH into tools.haiocloud.com to add domains at top, removes comments and duplicates, then syncs to tahrim.haiocloud.com:/etc/trojan/domains.txt via curl.
---

# Add Domains to tools.haiocloud.com

Use this skill when the user wants to add new domain entries to the Haio bypass domains list.

## Workflow

### Step 1: Add domains to tools.haiocloud.com

SSH into `tools.haiocloud.com` on port `2280` and prepend new domains to `/var/www/html/domains.txt`.

**Command pattern:**
```bash
ssh root@tools.haiocloud.com -p 2280 "sed -i '1i <domains-separated-by-newlines>' /var/www/html/domains.txt"
```

**Example with multiple domains:**
```bash
ssh root@tools.haiocloud.com -p 2280 "sed -i '1i example.com\ndomain2.com\napi.example.com' /var/www/html/domains.txt"
```

### Step 2: Remove comments and duplicates

After adding, clean the file by removing any comment lines (`# ...`) and deduplicating:

```bash
ssh root@tools.haiocloud.com -p 2280 "sed -i '/^#/d' /var/www/html/domains.txt && awk '!seen[\$0]++' /var/www/html/domains.txt > /tmp/domains_deduped && mv /tmp/domains_deduped /var/www/html/domains.txt"
```

### Step 3: Sync to tahrim.haiocloud.com

Download the updated file to the trojan server:

```bash
ssh root@tahrim.haiocloud.com "curl -s -o /etc/trojan/domains.txt https://tools.haiocloud.com/domains.txt"
```

### Step 4: Verify

Check both servers:

```bash
# Verify tools.haiocloud.com
ssh root@tools.haiocloud.com -p 2280 "head -20 /var/www/html/domains.txt && echo '---' && wc -l /var/www/html/domains.txt"

# Verify tahrim.haiocloud.com
ssh root@tahrim.haiocloud.com "head -20 /etc/trojan/domains.txt && echo '---' && wc -l /etc/trojan/domains.txt"
```

## Important Rules

- **Never add comment lines** (lines starting with `#`) to the file
- Always prepend new domains at the **top** of the file (line 1)
- Always run deduplication after adding
- Always sync to tahrim.haiocloud.com after updating tools.haiocloud.com
- Verify both servers after sync

## Server Details

| Server | SSH | File Path |
|--------|-----|-----------|
| tools.haiocloud.com | `ssh root@tools.haiocloud.com -p 2280` | `/var/www/html/domains.txt` |
| tahrim.haiocloud.com | `ssh root@tahrim.haiocloud.com` | `/etc/trojan/domains.txt` |
