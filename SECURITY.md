# Security Policy

## Supported Versions

| Version | Supported |
| :--- | :--- |
| `0.1.x` | :white_check_mark: |

Policy-gate integrity (`PolicyMode::ReadOnlyOnly` default returning JSON-RPC `-32003`), deterministic `dry_run` previews, safe Windows command/environment quoting, and strict R4 invariants (zero `RunAsDate`, zero DLL injection, zero API hooking, zero credential harvesting) are top priorities for `winctl2rust`.

If you discover a security vulnerability, policy-gate bypass, command-injection/quoting edge case, or R4 boundary violation:
1. **Do not disclose it publicly** via GitHub Issues.
2. Submit a report privately via GitHub Private Vulnerability Reporting under the Security tab:
   `https://github.com/adpaolucci/winctl2rust/security/advisories/new`
3. You will receive an acknowledgment within 48 hours.
4. A security patch and advisory will be coordinated and released promptly.
