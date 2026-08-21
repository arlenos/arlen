<!--
SPDX-FileCopyrightText: 2026 Tim Kicker

SPDX-License-Identifier: AGPL-3.0-only
-->

# Encrypted fixtures

Two one-page PDFs, both written by `qpdf --encrypt ... --bits=256` over a blank page, because the thing
under test is what a real encryptor produces and a hand-assembled `/Encrypt` dictionary would only test
somebody's idea of one.

  * `user-locked.pdf` - a user password (`secret`). Nobody opens this without it, and the reader must say
    so rather than calling the file damaged.
  * `owner-only.pdf` - an owner password and an EMPTY user password. Everybody opens this; it is the case
    a naive "is it encrypted" check would refuse by mistake.

To rebuild either:

    qpdf --encrypt --user-password=secret --owner-password=owner --bits=256 -- plain.pdf user-locked.pdf
    qpdf --encrypt --user-password=      --owner-password=owner --bits=256 -- plain.pdf owner-only.pdf
