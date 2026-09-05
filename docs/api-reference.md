# API Reference

Claude OAuth examples. Codex, Grok, and 9Router use different endpoints; see `src-tauri/src/sources/`.

> [!NOTE]
> Real-world examples with anonymized data, captured in March 2026. Undocumented internal endpoints. Fields may change without notice.

## /api/oauth/usage

```
https://api.anthropic.com/api/oauth/usage
```

```json
{
  "five_hour": {
    "utilization": 48.0,
    "resets_at": "2026-03-02T11:00:00.521744+00:00"
  },
  "seven_day": {
    "utilization": 64.0,
    "resets_at": "2026-03-06T06:00:00.521764+00:00"
  },
  "seven_day_oauth_apps": null,
  "seven_day_opus": null,
  "seven_day_sonnet": {
    "utilization": 2.0,
    "resets_at": "2026-03-06T07:00:00.521773+00:00"
  },
  "seven_day_cowork": null,
  "iguana_necktie": null,
  "extra_usage": {
    "is_enabled": true,
    "monthly_limit": 1000,
    "used_credits": 0.0,
    "utilization": null
  }
}
```

## /api/oauth/profile

```
https://api.anthropic.com/api/oauth/profile
```

```json
{
  "account": {
    "uuid": "...",
    "full_name": "Max Clau",
    "display_name": "Max",
    "email": "max@clau.de",
    "has_claude_max": true,
    "has_claude_pro": false,
    "created_at": "2024-10-22T07:21:47.099776Z"
  },
  "organization": {
    "uuid": "...",
    "name": "max@clau.de's Organization",
    "organization_type": "claude_max",
    "billing_type": "stripe_subscription",
    "rate_limit_tier": "default_claude_max_5x",
    "has_extra_usage_enabled": true,
    "subscription_status": "active",
    "subscription_created_at": "2026-01-16T18:22:42.826732Z"
  },
  "application": {
    "uuid": "...",
    "name": "Claude Code",
    "slug": "claude-code"
  }
}
```
