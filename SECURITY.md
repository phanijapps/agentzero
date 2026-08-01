# Security Policy

## Reporting Security Vulnerabilities

**IMPORTANT:** Please do **NOT** create public GitHub issues for security vulnerabilities.

### How to Report

If you discover a security vulnerability in z-bot, please send an email to:

- **Email:** `security@zbot.dev` (or replace with actual security email)

Please include:

1. A description of the vulnerability
2. Steps to reproduce the issue
3. Potential impact assessment
4. Suggested mitigation (if known)

### Response Timeline

| Severity Level | Initial Response | Fix Timeline |
|---------------|-----------------|--------------|
| **Critical** | 24 hours | 48 hours |
| **High** | 48 hours | 7 days |
| **Medium** | 3 days | 14 days |
| **Low** | 7 days | 30 days |

---

## Supported Versions

| Version | Security Updates |
|---------|------------------|
| Main branch (`main`) | ✅ Yes |
| Latest release | ✅ Yes |
| Releases older than 3 months | ❌ No |

---

## Current Security Posture

### ✅ Strengths

| Area | Status |
|------|--------|
| **Memory Safety** | Rust's memory model prevents buffer overflows, use-after-free |
| **SQL Injection** | All database queries use parameterized statements |
| **XSS Prevention** | React's built-in escaping; no `innerHTML` or `dangerouslySetInnerHTML` usage |
| **Dependency Scanning** | CI includes `cargo audit`, `npm audit`, Gitleaks |
| **Shell Guardrails** | 90+ blocked dangerous commands in shell tool |

### ⚠️ Known Limitations

| Issue | Severity | Status |
|-------|----------|--------|
| No authentication/authorization | 🔴 Critical | Documented |
| No TLS/HTTPS support | 🔴 High | Documented |
| No rate limiting | 🟠 High | Documented |
| API keys stored in plaintext | 🟠 High | Documented |
| No security headers | 🟡 Medium | Documented |
| Unmaintained dependencies (2) | 🟡 Medium | Tracking |

---

## Security Architecture

### Data Flow

```
┌─────────────┐         ┌─────────────┐         ┌─────────────┐
│   Client    │◄────────│  Gateway    │◄────────│  LLM APIs   │
│ (Web/CLI)   │ HTTP+WS │  (Axum)     │  HTTPS   │ (External)  │
└─────────────┘ :18791  └─────────────┘         └─────────────┘
                      │
                      │ Data
                      │
                  ┌───┴────┐
                  │ SQLite │
                  │ (Local)│
                  └────────┘
```

### Trust Boundaries

1. **Local-first:** All data stored locally on user's machine
2. **No cloud dependency:** Core functionality works offline
3. **External APIs:** LLM providers require API keys (user-provided)

---

## Dependency Security

### Automated Scanning

| Tool | Purpose | Frequency |
|------|---------|-----------|
| `cargo audit` | Rust vulnerability scanning | Every PR + Weekly |
| `cargo deny` | License + bans + advisories | Every PR |
| `npm audit` | Node.js vulnerabilities | Every PR + Weekly |
| `Gitleaks` | Secrets scanning | Every PR |

### Current Vulnerabilities

**Active CVEs/RUSTSEC:**
- See [Security Scan Report](docs/security/scan-report-YYYY-MM-DD.md) for latest findings

**Unmaintained Dependencies:**
- `instant` 0.1.13 → replacement: `web-time`
- `paste` 1.0.15 → replacement: `pastey` or `with_builtin_macros`

---

## Security Checklist for Deployment

### Development Environment
- [ ] Run `cargo deny check` locally
- [ ] Run `cargo audit` locally
- [ ] Run `npm audit` in `apps/ui`
- [ ] Run `gitleaks detect` locally
- [ ] Review `unwrap()` usage for potential panics

### Production Deployment (Future)

**Before deploying to production:**

- [ ] **Authentication**: Implement API authentication (JWT, OAuth2)
- [ ] **TLS/HTTPS**: Enable HTTPS with valid certificates
- [ ] **Rate Limiting**: Configure per-IP/user rate limits
- [ ] **Security Headers**: Add CSP, HSTS, X-Frame-Options
- [ ] **Secrets Encryption**: Encrypt API keys at rest
- [ ] **Audit Logging**: Enable security event logging
- [ ] **Input Validation**: Review all user input handling
- [ ] **CORS Policy**: Restrict to specific origins (not `*`)

---

## Threat Model

### Assumptions

1. **Trusted Local Environment:** User controls their machine
2. **Untrusted Network:** Network traffic may be intercepted
3. **Untrusted LLM Providers:** External APIs may be compromised

### Threat Actors

| Actor | Capability | Mitigation |
|-------|------------|------------|
| **Network Attacker** | MitM, packet sniffing | Future: TLS |
| **Local User** | File system access | File permissions |
| **Malicious Agent** | Arbitrary code execution | Shell guardrails |
| **Compromised LLM API** | Data exfiltration | Audit logs |

### Attack Surface

| Component | Exposure | Controls |
|-----------|----------|----------|
| HTTP API + static UI (`:18791`) | LAN by default; configurable | Bind host, firewall, route-specific validation |
| Client event WebSocket (`:18791/ws`) | Shares the HTTP listener | Configurable bind host, typed client protocol |
| Bridge worker WebSocket (`:18791/bridge/ws`) | Shares the HTTP listener | Configurable bind host, typed Hello handshake with timeout |
| Shell Tool | Command execution | Guardrails, sandboxing |
| File Tools | File read/write | Path validation |
| LLM Integration | External API calls | API key isolation |

The effective gateway bind address comes from network settings: `advanced.bindHost`
takes precedence; otherwise `network.exposeToLan=true` (the default) binds all
interfaces, while `false` binds loopback only. Both WebSocket routes share that
listener. The gateway does not currently provide transport TLS or built-in client
authentication, so LAN deployments must use a trusted network, firewall, or an
authenticating TLS reverse proxy.

---

## Security Best Practices

### For Users

1. **Never share API keys** in agent configurations
2. **Review agent permissions** before executing
3. **Keep dependencies updated** with `cargo update`
4. **Run security scans** before committing code
5. **Use firewall rules** to restrict local port access

### For Developers

1. **Follow the security checklist** before merging code
2. **Use `cargo deny`** to catch license issues
3. **Avoid `unwrap()`** in production code paths
4. **Validate all user input** before processing
5. **Log security events** for audit trails
6. **Review dependency updates** for vulnerabilities

---

## Security Testing

### Automated Tests

```bash
# Run all security checks
cargo deny check
cargo audit
npm audit

# Run tests
cargo test
npm test
```

### Manual Testing Checklist

- [ ] Test shell command guardrails
- [ ] Verify file path traversal protection
- [ ] Test API endpoint access controls
- [ ] Validate CORS configuration
- [ ] Review error messages for information leakage

---

## Security Changelog

### 2026-02-03
- Initial security assessment completed
- `cargo-deny` configuration added
- `SECURITY.md` created
- Identified: 1 CVE, 2 unmaintained dependencies
- Documented: No authentication, no TLS, plaintext secrets

---

## Resources

- [Rust Security Guidelines](https://doc.rust-lang.org/book/ch12-00-anatomy.html)
- [OWASP Rust Top 10](https://owasp.org/www-project-rust-security/)
- [Cargo Audit Documentation](https://github.com/RustSec/cargo-audit)
- [Cargo Deny Documentation](https://embarkstudios.github.io/cargo-deny/)
- [React Security](https://react.dev/learn/keeping-components-pure)

---

## License

This security policy is part of the z-bot project and is licensed under the same terms as the main project (MIT License).
