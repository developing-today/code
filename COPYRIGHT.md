## Contributing Information

By submitting patches to this project you agree to allow them to be redistributed under the project's license, according to the normal forms and usages of the open-source community.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as below, without any additional terms or conditions.

Copyrights in this project are retained by the contributors to the project.

No copyright assignment is required to contribute to this project.

## Licensing Information

Copyright developing.today LLC & contributors to the project.

This project is licensed under either of

- [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0) ([`LICENSE-APACHE`](LICENSE-APACHE) in the root directory of this source tree)
- [MIT license](https://opensource.org/licenses/MIT) ([`LICENSE-MIT`](LICENSE-MIT) in the root directory of this source tree)

at your option.

The [SPDX](https://spdx.dev) license identifier for this project is `MIT OR Apache-2.0`.

Except as otherwise noted below and/or in [`README.md`](README.md) and/or in [`NOTICE`](NOTICE) and/or in individual files and/or in individual directories.

## Acknowledgements

A partial list of copyright acknowledgements related to this project.

- within [`build/`](build)
  - for nix-related works within [`build/nix`](build/nix) and [`build/buck/preludes/nix-prelude`](build/buck/preludes/nix-prelude)
    - SPDX-License-Identifier: MIT OR Apache-2.0
    - SPDX-FileCopyrightText: © 2022-2023 Austin Seipp

## Boilerplate Information

### How to apply the licensing to your work:

Except as otherwise noted, such as differently-licensed or vendored code,

(below and/or in [`README.md`](README.md) and/or in [`NOTICE`](NOTICE) and/or in individual files and/or in individual directories,)

**A standard header should be applied to any files added to this source tree.**

To apply the License to your work, attach the following boilerplate notice.

The text should be enclosed in the appropriate comment syntax for the file format.

We also recommend that a file or class name and description of purpose be included

on the same "printed page" as the copyright notice for easier identification

within third-party archives.

```
Copyright (c) developing.today LLC & contributors to the project.

This source code is licensed under both the MIT license found in the
LICENSE-MIT file in the root directory of this source tree and the Apache
License, Version 2.0 found in the LICENSE-APACHE file in the root directory
of this source tree.

SPDX-License-Identifier: MIT OR Apache-2.0
```

A lack of boilerplate notice on a file does not imply anything about its copyrights or licensing.

Lack of a boilerplate notice could be any of the following reasons:

- Code-specific reasons the file cannot support commenting.
- Binary files
- Conciseness for specific kinds of files, configurations, etc.
- An oversight
- An aesthetic choice
- any other reason

All files retain all applicable copyrights and licenses, without regard for boilerplate notice.

## Sign-off

Certify your work by adding your `Signed-off-by` trailer to your commit.

To improve tracking of who did what, we ask you to certify that you

wrote the patch or have the right to pass it on under the same license

as ours, by "signing off" your patch. Without sign-off, we cannot

accept your patches.

We prefer you sign-off every commit, but need you to sign-off

at least one commit in each patch you send us. For larger patches,

at the very least, the last commit must be signed off.

If (and only if) you certify the below DCO:

The sign-off is a simple line at the end of the explanation for the
patch,

which certifies that you wrote it or otherwise have the right to
pass it on as an open-source patch.

The rules are pretty simple:

if you can certify the below (from [developercertificate.org](https://developercertificate.org/)


also located at [`DCO-1.1`](DCO-1.1) in the root directory of this source tree):

### DCO - Developer Certificate of Origin, Version 1.1

```
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.

Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
have the right to submit it under the open source license
indicated in the file; or

(b) The contribution is based upon previous work that, to the best
of my knowledge, is covered under an appropriate open source
license and I have the right under that license to submit that
work with modifications, whether created in whole or in part
by me, under the same open source license (unless I am
permitted to submit under a different license), as indicated
in the file; or

(c) The contribution was provided directly to me by some other
person who certified (a), (b) or (c) and I have not modified
it.

(d) I understand and agree that this project and the contribution
are public and that a record of the contribution (including all
personal information I submit with it, including my sign-off) is
maintained indefinitely and may be redistributed consistent with
this project or the open source license(s) involved.
```

you add a "Signed-off-by" trailer to your commit, that looks like
this:

```
Signed-off-by: Random J Developer <random@developer.example.org>
```

This line can be added by Git if you run `git commit` with `-s` option: `git commit -s`

You can also add the trailer manually to your commit message.

Notice that you can place your own `Signed-off-by` trailer

when
forwarding somebody else's patch with the above rules for
DCO.

Indeed you are encouraged to do so.

Do not forget to
place an in-body `From: ` commit header line at the beginning

to properly attribute the change to its true author (see (2) above).

This procedure originally came from the Linux kernel project, so our
rule is quite similar to theirs,

but what exactly it means to sign-off your patch differs from project to project,

so it may be different from that of the project you are accustomed to.

### SoB Chain

Any further SoBs (Signed-off-by:'s) following the author's SoB are from

people handling and transporting the patch, but were not involved in its

development. SoB chains should reflect the **real** route a patch took

as it was propagated to the maintainers and ultimately merged, with

the first SoB entry signalling primary authorship of a single author.

### Real Identity

Also notice that a real identity is used in the `Signed-off-by` trailer.

Please don't hide your real identity,

please email a maintainer if this is a concern.

### Commit Trailers

If you like, you can put extra tags at the end:

. `Reported-by:` is used to credit someone who found the bug that
the patch attempts to fix.

. `Acked-by:` says that the person who is more familiar with the area
the patch attempts to modify liked the patch.

. `Reviewed-by:`, unlike the other tags, can only be offered by the
reviewers themselves when they are completely satisfied with the
patch after a detailed analysis.

. `Tested-by:` is used to indicate that the person applied the patch
and found it to have the desired effect.

You can also create your own tag or use one that's in common usage
such as "Thanks-to:", "Based-on-patch-by:", or "Mentored-by:".

Do not forget to add trailers such as `Acked-by:`, `Reviewed-by:`

and `Tested-by:` lines as necessary to credit people who helped your patch,

and "cc:" them when sending such a final version for inclusion.

A Suggested-by: tag indicates that the patch idea is suggested by the person
named and ensures credit to the person for the idea. Please note that this
tag should not be added without the reporter's permission, especially if the
idea was not posted in a public forum. That said, if we diligently credit our
idea reporters, they will, hopefully, be inspired to help us again in the
future.

A Fixes: tag indicates that the patch fixes an issue in a previous commit. It
is used to make it easy to determine where a bug originated, which can help
review a bug fix. This tag also assists the team in determining
which stable versions should receive your fix. This is the preferred
method for indicating a bug fixed by the patch.
