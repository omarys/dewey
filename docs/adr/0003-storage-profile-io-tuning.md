# 3. Storage Profile Adaptive I/O Tuning

We introduced runtime storage profiles (`Fast` vs `Usb`) to automatically adapt SQLite memory mapping, scanner thread concurrency, and companion reader pre-buffering to the storage medium. On slow or removable flash storage (USB thumb drives, SD cards), high thread concurrency causes controller queue thrashing and `mmap` can trigger page fault stalls on filesystems like exFAT. Setting `StorageProfile::Usb` clamps scanner concurrency to 2 threads, disables SQLite `mmap_size`, and signals Continuum to increase chapter preloading lookahead.
