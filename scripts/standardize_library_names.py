#!/usr/bin/env python3
"""
Standardize Dewey Library Archive Filenames.

Converts legacy bracket-prefixed comic archives (e.g. `[0009]_Chapter_0.9_The_Golden_Age_(1).cbz`)
into standardized clean naming: `<Series Title> - Chapter <Num> - <Title>.cbz`.

Safely updates Dewey's SQLite database (`chapters.file_path`) in an atomic transaction so reading
progress and bookmarks remain intact.

Defaults to --dry-run for safety. Use --apply to execute changes.
"""

import argparse
import json
import os
import re
import shutil
import sqlite3
import sys
import time
from pathlib import Path

DEFAULT_CONFIG_PATH = Path.home() / ".config" / "dewey" / "config.toml"
DEFAULT_LIBRARY_DIR = Path.home() / "Documents" / "Dewey"
DEFAULT_DB_PATH = Path.home() / ".local" / "share" / "dewey" / "dewey.db"


def load_dewey_config():
    library_dir = DEFAULT_LIBRARY_DIR
    db_path = DEFAULT_DB_PATH

    if DEFAULT_CONFIG_PATH.exists():
        try:
            with open(DEFAULT_CONFIG_PATH, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if line.startswith("library_dir"):
                        val = line.split("=", 1)[1].strip().strip("\"'")
                        library_dir = Path(os.path.expanduser(val))
                    elif line.startswith("db_path"):
                        val = line.split("=", 1)[1].strip().strip("\"'")
                        db_path = Path(os.path.expanduser(val))
        except Exception as e:
            print(f"[WARN] Failed reading {DEFAULT_CONFIG_PATH}: {e}", file=sys.stderr)

    return library_dir, db_path


def get_series_title(dir_path: Path) -> str:
    json_path = dir_path / "series.json"
    if json_path.exists():
        try:
            with open(json_path, "r", encoding="utf-8") as f:
                data = json.load(f)
                meta = data.get("metadata", data)
                name = meta.get("name") or meta.get("title")
                if name:
                    return name.strip()
        except Exception:
            pass
    raw = dir_path.name
    return raw.lstrip(".").replace("_", " ").strip()


def transform_filename(series_title: str, filename: str) -> str | None:
    stem, ext = os.path.splitext(filename)
    if ext.lower() not in (".cbz", ".zip", ".cbr", ".epub", ".pdf"):
        return None

    # Already follows "<Series> - " format
    if filename.startswith(f"{series_title} - "):
        return None

    m = re.match(r"^\[\d+\]_(.*)$", stem)
    if not m:
        return None
    rest = m.group(1)

    m2 = re.match(
        r"^(Chapter|Episode|Vol\.?|Volume)[_.]*([0-9]+(?:\.[0-9]+)?)(?:_(.*))?$",
        rest,
        re.IGNORECASE,
    )
    if m2:
        kind = m2.group(1).capitalize()
        if kind.lower().startswith("vol"):
            kind = "Vol."
        num = m2.group(2)
        subtitle = m2.group(3) or ""

        # Remove trailing Last_Read or timestamps
        subtitle = re.sub(r"Last_Read.*$", "", subtitle, flags=re.IGNORECASE)
        # Remove redundant chapter repetitions e.g. "Ch._30_-_Title"
        subtitle = re.sub(
            r"^(Ch\.|Chapter|Episode)[_.\s]*[0-9.]*[-_\s]*",
            "",
            subtitle,
            flags=re.IGNORECASE,
        )
        subtitle = subtitle.replace("_", " ").strip()
        subtitle = re.sub(r"\s+", " ", subtitle).strip()
        subtitle = subtitle.strip(" -")

        if subtitle:
            new_stem = f"{series_title} - {kind} {num} - {subtitle}"
        else:
            new_stem = f"{series_title} - {kind} {num}"
        return new_stem + ext
    else:
        clean_rest = rest.replace("_", " ").strip()
        clean_rest = re.sub(r"\s+", " ", clean_rest).strip()
        return f"{series_title} - {clean_rest}{ext}"


def main():
    cfg_lib, cfg_db = load_dewey_config()

    parser = argparse.ArgumentParser(
        description="Standardize Dewey comic archive filenames and update SQLite database."
    )
    parser.add_argument(
        "--library-dir",
        type=Path,
        default=cfg_lib,
        help=f"Path to Dewey library directory (default: {cfg_lib})",
    )
    parser.add_argument(
        "--db-path",
        type=Path,
        default=cfg_db,
        help=f"Path to Dewey SQLite database (default: {cfg_db})",
    )
    parser.add_argument(
        "--series",
        type=str,
        default="",
        help="Optional series title or folder name filter (e.g. 'Berserk')",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help="Execute the file renames and database updates (default is dry-run)",
    )
    parser.add_argument(
        "--no-backup",
        action="store_true",
        help="Skip database backup before applying changes",
    )

    args = parser.parse_args()
    is_dry_run = not args.apply

    if is_dry_run:
        print("=== RUNNING IN DRY-RUN MODE (No files or database records will be modified) ===")
        print("Use --apply to perform actual renaming.\n")
    else:
        print("=== APPLYING CHANGES TO FILESYSTEM AND DATABASE ===\n")

    if not args.library_dir.exists():
        print(f"Error: Library directory not found: {args.library_dir}", file=sys.stderr)
        sys.exit(1)

    conn = None
    if args.db_path.exists():
        try:
            conn = sqlite3.connect(args.db_path)
        except Exception as e:
            print(f"[WARN] Could not connect to database {args.db_path}: {e}", file=sys.stderr)

    total_series = 0
    total_renames = 0
    total_skipped = 0
    total_collisions = 0
    db_updates = []

    series_dirs = []
    for root, dirs, files in os.walk(args.library_dir):
        has_cbz = any(f.lower().endswith((".cbz", ".zip", ".cbr")) for f in files)
        if has_cbz or "series.json" in files:
            series_dirs.append(Path(root))

    if args.series:
        filter_lower = args.series.lower()
        series_dirs = [
            d for d in series_dirs
            if filter_lower in d.name.lower() or filter_lower in get_series_title(d).lower()
        ]

    series_dirs.sort()

    for s_dir in series_dirs:
        series_title = get_series_title(s_dir)
        try:
            entries = sorted(os.listdir(s_dir))
        except Exception as e:
            print(f"[ERROR] Could not read {s_dir}: {e}", file=sys.stderr)
            continue

        existing_names = set(entries)
        planned_names = set()
        series_renames = []

        for f in entries:
            new_name = transform_filename(series_title, f)
            if not new_name:
                total_skipped += 1
                continue

            if new_name in existing_names or new_name in planned_names:
                print(f"[COLLISION] Skipping {f} -> {new_name} (already exists in {s_dir.name})", file=sys.stderr)
                total_collisions += 1
                continue

            planned_names.add(new_name)
            src_path = s_dir / f
            dst_path = s_dir / new_name
            series_renames.append((src_path, dst_path))

        if series_renames:
            total_series += 1
            print(f"\n📂 [{series_title}] ({len(series_renames)} files to standardize):")
            for src, dst in series_renames[:5]:
                print(f"   {src.name}\n      -> {dst.name}")
            if len(series_renames) > 5:
                print(f"   ... and {len(series_renames) - 5} more")

            for src, dst in series_renames:
                total_renames += 1
                db_updates.append((str(dst), str(src)))
                if not is_dry_run:
                    os.rename(src, dst)

    print("\n" + "=" * 60)
    print(f"Summary:")
    print(f"  Series inspected:     {len(series_dirs)}")
    print(f"  Series to update:     {total_series}")
    print(f"  Files to standardize: {total_renames}")
    print(f"  Files already clean:  {total_skipped}")
    print(f"  Collisions detected:  {total_collisions}")

    if not is_dry_run and total_renames > 0 and conn:
        if not args.no_backup:
            backup_path = args.db_path.with_name(f"{args.db_path.name}.bak.{int(time.time())}")
            shutil.copy2(args.db_path, backup_path)
            print(f"  Database backup created: {backup_path}")

        cur = conn.cursor()
        updated_rows = 0
        for new_path, old_path in db_updates:
            cur.execute("UPDATE chapters SET file_path = ? WHERE file_path = ?", (new_path, old_path))
            updated_rows += cur.rowcount

        conn.commit()
        conn.close()
        print(f"  Database records updated: {updated_rows}")
        print("\n✅ Successfully standardized library filenames and updated database.")
    elif is_dry_run and total_renames > 0:
        print("\n💡 Run with --apply to execute these renames.")


if __name__ == "__main__":
    main()
