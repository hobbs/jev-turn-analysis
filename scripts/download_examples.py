#!/usr/bin/env python3
"""Fetch only pinned public examples; never access native personal session paths."""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-only", action="store_true", help="Verify local fixture checksums without network")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parents[1] / "tests/fixtures/public"
    records = json.loads((directory / "PROVENANCE.json").read_text())
    pending = []
    for record in records:
        path = directory / record["file"]
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == record["sha256"]:
            print(f"verified {record['file']}")
        elif args.verify_only:
            raise SystemExit(f"missing or checksum mismatch: {path}")
        else:
            pending.append(record)
    by_url = {}
    for record in pending:
        by_url.setdefault(record["source_url"], []).append(record)
    for url, group in by_url.items():
        with urllib.request.urlopen(url, timeout=120) as response:
            if "source_record_index" in group[0]:
                wanted = {r["source_record_index"]: r for r in group}
                for index, line in enumerate(response):
                    if index in wanted:
                        record = wanted.pop(index)
                        data = (json.dumps(json.loads(line), ensure_ascii=False, separators=(",", ":")) + "\n").encode()
                        save(directory, record, data)
                    if not wanted:
                        break
                if wanted:
                    raise SystemExit("dataset ended before selected records")
            else:
                save(directory, group[0], response.read())


def save(directory, record, data):
    if hashlib.sha256(data).hexdigest() != record["sha256"]:
        raise SystemExit(f"checksum mismatch from pinned source: {record['file']}")
    path = directory / record["file"]
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)
    print(f"downloaded and verified {record['file']}")


if __name__ == "__main__":
    main()
