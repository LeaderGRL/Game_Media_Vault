# Render PDF previews with pdfium, loaded from explicit directories

Manuals and other documents are mostly PDF files, and the Library needs to see them as more than file names. Game Media Vault renders their previews in the application with pdfium, the PDF engine of Chromium: the thumbnail recipe draws a PDF original's first page to fit the longest edge, and stores the result as a Derived Asset like any other thumbnail. The original bytes stay untouched.

pdfium is a native library that the application loads at run time through `pdfium-render`, so the build needs no PDF engine and a machine without one still runs, skipping PDF originals. The library is loaded only from explicit directories: the one `GAME_MEDIA_VAULT_PDFIUM` names, then the one holding the running executable. It is never loaded by bare name through the system search path, since on Windows that path includes the working directory and `PATH`, where a planted library would run inside the application. Binding happens once per process.

Rendering in the application keeps the previews identical on every machine and for every frontend, unlike a webview's PDF viewer, which differs between platforms and could not produce Derived Assets. Shipping the pdfium library with release builds is a separate decision about where its binary comes from.
