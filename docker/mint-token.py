#!/usr/bin/env python3
"""Bootstrap the OAuth material and mint RS256 access tokens for MeatShell.

MeatShell is an OAuth *resource server*: it only verifies RS256 tokens against
a public JWKS. There is no built-in authorization server. This helper covers
the two things an operator needs on a private deployment:

  init    generate a signing key, the public JWKS and a matching http.json
  mint    sign a short-lived access token for a subject

Examples:

    # one-time bootstrap (writes config/http.json + config/public-jwks.json
    # and the private key, refusing to overwrite existing files)
    python3 docker/mint-token.py init \
        --config-dir config --key secrets/jwt-key.pem \
        --issuer https://meatshell.local/ \
        --resource https://shell.example.com/mcp \
        --subject mrqiang

    # print a token valid for 5 minutes
    python3 docker/mint-token.py mint --key secrets/jwt-key.pem \
        --issuer https://meatshell.local/ \
        --resource https://shell.example.com/mcp \
        --subject mrqiang --ttl 300

The token goes in `Authorization: Bearer <token>`; keep the private key out of
the repository and off any shared storage.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import stat
import sys
import time
from pathlib import Path

try:
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import padding, rsa
except ImportError:  # pragma: no cover
    sys.exit("mint-token: needs python3-cryptography (pip install cryptography)")

SCOPE = "meatshell:mcp"
KID = "meatshell-local"


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def write_if_absent(path: Path, text: str, mode: int = 0o644) -> None:
    if path.exists():
        sys.exit(f"mint-token: refusing to overwrite {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    os.chmod(path, mode)


def cmd_init(args: argparse.Namespace) -> None:
    key_path = Path(args.key)
    config_dir = Path(args.config_dir)
    for target in (key_path, config_dir / "public-jwks.json", config_dir / "http.json"):
        if target.exists():
            sys.exit(f"mint-token: refusing to overwrite {target}")

    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    key_path.parent.mkdir(parents=True, exist_ok=True)
    key_path.write_bytes(
        key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.PKCS8,
            encryption_algorithm=serialization.NoEncryption(),
        )
    )
    os.chmod(key_path, stat.S_IRUSR | stat.S_IWUSR)

    numbers = key.public_key().public_numbers()
    jwks = {
        "keys": [
            {
                "kty": "RSA",
                "kid": KID,
                "alg": "RS256",
                "use": "sig",
                "n": b64url(numbers.n.to_bytes((numbers.n.bit_length() + 7) // 8, "big")),
                "e": b64url(numbers.e.to_bytes((numbers.e.bit_length() + 7) // 8, "big")),
            }
        ]
    }
    write_if_absent(config_dir / "public-jwks.json", json.dumps(jwks, indent=2) + "\n")

    config = {
        "bind": args.bind,
        "allowed_origins": [],
        "oauth": {
            "issuer": args.issuer,
            "resource": args.resource,
            "jwks_file": "public-jwks.json",
            "required_scope": SCOPE,
            "allowed_subjects": args.subject,
        },
    }
    write_if_absent(config_dir / "http.json", json.dumps(config, indent=2) + "\n")

    print(f"private key   {key_path} (mode 600, keep it private)")
    print(f"public JWKS   {config_dir / 'public-jwks.json'}")
    print(f"http config   {config_dir / 'http.json'}")
    print(f"issuer        {args.issuer}")
    print(f"resource      {args.resource}")
    print(f"subjects      {', '.join(args.subject)}")


def cmd_mint(args: argparse.Namespace) -> None:
    key = serialization.load_pem_private_key(Path(args.key).read_bytes(), password=None)
    now = int(time.time())
    header = {"alg": "RS256", "kid": KID, "typ": "JWT"}
    claims = {
        "iss": args.issuer,
        "aud": args.resource,
        "sub": args.subject,
        "scope": args.scope,
        "iat": now,
        "exp": now + args.ttl,
    }
    signing_input = ".".join(
        b64url(json.dumps(part, separators=(",", ":")).encode())
        for part in (header, claims)
    ).encode()
    signature = key.sign(signing_input, padding.PKCS1v15(), hashes.SHA256())
    print(signing_input.decode() + "." + b64url(signature))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    init = sub.add_parser("init", help="generate key, JWKS and http.json")
    init.add_argument("--config-dir", default="config")
    init.add_argument("--key", default="secrets/jwt-key.pem")
    init.add_argument("--issuer", default="https://meatshell.local/")
    init.add_argument("--resource", default="https://shell.example.com/mcp")
    init.add_argument("--bind", default="0.0.0.0:8765")
    init.add_argument("--subject", action="append", required=True,
                      help="allowed subject; repeat for more")
    init.set_defaults(func=cmd_init)

    mint = sub.add_parser("mint", help="print a bearer token")
    mint.add_argument("--key", default="secrets/jwt-key.pem")
    mint.add_argument("--issuer", required=True)
    mint.add_argument("--resource", required=True)
    mint.add_argument("--subject", required=True)
    mint.add_argument("--scope", default=SCOPE)
    mint.add_argument("--ttl", type=int, default=300)
    mint.set_defaults(func=cmd_mint)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
