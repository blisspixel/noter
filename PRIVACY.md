# Noter Data Bill of Rights (Privacy Policy)

**Effective Date: July 2026**

Noter is built on a fundamental belief: your thoughts, notes, and code belong to you. We reject the normalization of telemetry, "anonymous" data harvesting, and cloud-forced AI integration in personal utilities.

To that end, this document serves as a binding technical and philosophical contract.

## 1. Zero Network Activity
The official Noter binary contains **zero network calls**. It will never "phone home," it does not check for updates, it does not send crash reports, and it does not download assets at runtime.

## 2. Zero Telemetry
We do not track:
- How often you open the app.
- What features you use.
- The size, type, or location of your files.
- Your OS version or hardware specifics.

"Anonymous diagnostics" are often a backdoor to behavioral profiling. We reject this entirely. If the app crashes, it is up to you to voluntarily submit a bug report via GitHub. 

## 3. Total Data Sovereignty
Noter will never read a file on your disk that you did not explicitly open via the file dialog or command line. 
Autosave and recovery files are stored strictly locally in your OS's temporary directory and are never synchronized to our servers.

## 4. No AI Training
Your private data is yours. Because Noter has no network access, it is technically impossible for us to ingest your writing to train language models, build behavioral profiles, or "enhance our services."

---

*If you ever compile Noter from source and find a network request hidden in the core codebase, consider it a critical security vulnerability and a breach of this contract.*
