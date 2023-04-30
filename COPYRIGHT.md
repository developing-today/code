## Contributing Information

By submitting patches to this project you agree to allow them to be redistributed under the project's license,

according to the normal forms and usages of the open-source community.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,

as defined in the Apache-2.0 license, shall be dual-licensed as below, without any additional terms or conditions.

Copyrights in this project are retained by the contributors to the project.

No copyright assignment is required to contribute to this project.

By committing to this project, you agree to the [Developer Certificate of Origin, Version 1.1](https://developercertificate.org/)

([`DCO-1.1`](DCO-1.1) in the root directory of this source tree.)

Please ensure to certify that your patch set adheres to these rules,

and has been "Signed-off-by" according to the guidelines in [`COPYRIGHT.md`](COPYRIGHT.md) in the root directory of this source tree.

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

## License Header Information

### How to apply the licensing to your work:

Except as otherwise noted, such as differently-licensed or vendored code,

(below and/or in [`README.md`](README.md) and/or in [`NOTICE`](NOTICE) and/or in individual files and/or in individual directories,)

**A standard header should be applied to any files added to this source tree.**

To apply the License to your work, attach the following boilerplate license notice.

The text should be enclosed in the appropriate comment syntax for the file format.

We also recommend that a file or class name and description of purpose be included

on the same "printed page" as the copyright notice for easier identification within third-party archives.

```
Copyright developing.today LLC & contributors to the project.

This source code is licensed under both the MIT license found in the
LICENSE-MIT file in the root directory of this source tree and the Apache
License, Version 2.0 found in the LICENSE-APACHE file in the root directory
of this source tree.

SPDX-License-Identifier: MIT OR Apache-2.0
```

A lack of boilerplate license notice on a file does not imply anything about its copyrights or licensing.

Lack of a boilerplate license notice could be any of the following reasons:

- Code-specific reasons the file cannot support commenting.
- Binary files.
- Conciseness for specific kinds of files, configurations, etc.
- An oversight.
- An aesthetic choice.
- Any other reason.

All files retain all applicable copyrights and licenses, without regard for boilerplate license notice.

## Certify to the Developer Certificate of Origin

By committing to this project, you certify that your work adheres to the [Developer Certificate of Origin, Version 1.1](https://developercertificate.org/)

([`DCO-1.1`](DCO-1.1) in the root directory of this source tree.)

Please ensure to certify that your patch set adheres to these rules,

and has been "Signed-off-by" according to the guidelines in [`COPYRIGHT.md`](COPYRIGHT.md) in the root directory of this source tree.

Please only "Signed-off-by" if, and only if, you can certify to the terms of the DCO.

Certify your work adheres to the DCO by adding your `Signed-off-by` trailer to your commit.

To improve tracking of who did what, we ask you to certify that you wrote the patch

or have the right to pass it on under the same license as ours, by "signing off" your patch.

Without sign-off, we cannot accept your patches.

We prefer you sign-off every commit, but need you to sign-off

at least one commit in each patch you send us. For larger patches,

at the very least, the last commit message must be signed off explicitly certifying `<first-sha>..<last-sha>`.

If (and only if) you certify the below DCO:

The sign-off is a simple line at the end of the explanation for the patch,

which certifies that you wrote it or otherwise have the right to pass it on as an open-source patch.

The rules are pretty simple:

if you can certify the below (from [developercertificate.org](https://developercertificate.org/)

also located at [`DCO-1.1`](DCO-1.1) in the root directory of this source tree):

### DCO - Developer Certificate of Origin, Version 1 . 1

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

you add a `Signed-off-by` trailer to your commit, that looks like
this:

```
Signed-off-by: Random J Developer <random@developer.example.org>
```

Please only "Signed-off-by" if, and only if, you can certify to the terms of the DCO.

This line can be added by Git if you run `git commit` with `-s` option: `git commit -s`

You can also add the trailer manually to your commit message.

Notice that you can place your own `Signed-off-by` trailer

when forwarding somebody else's patch with the above rules for DCO.

Indeed you are encouraged to do so.

Do not forget to place an in-body `From: ` commit header line at the beginning

to properly attribute the change to its true author (see (2) above).

This procedure originally came from the Linux kernel project,

so our rule is quite similar to theirs,

but what exactly it means to sign-off your patch differs from project to project,

so it may be different from that of the project you are accustomed to.

Below in this document are steps for adding your name to [`.mailmap`](.mailmap).

Adding your name to `.mailmap` certifies that

all commits you make in this organization will adhere to [`DCO-1.1`](DCO-1.1).

### Retroactive "Signed-off-by"

A quick way to certify a PR you forgot to "Signed-off-by" is as follows:

- Go to the PR
- Copy the the first sha in the chain you are prepared to certify.
  - Usually the first commit in your PR.
- Edit any file in the PR diff from within the PR UI.
- Commit the edit back, with a certification message:
  - 1 
```
I certify from <first-sha> to this commit.
```
  - or 2
```
certify: <first-sha>..<last-sha>
```
(in the second, `<last-sha>` is optional, but `..` is not.

no spaces next to `..`. you can space separated include other commits or commit chains.

When doing this, the commit back will say 'Sign off & commit'.

This means it will add your "Signed-off-by" for you,

so only do this if you can certify that all the commits

in the chain adhere to [`DCO-1.1`](DCO-1.1).

The same technique works manully from the command line,

however you will need to add "Signed-off-by" yourself

in the commit message or with `git commit -s` or some other way,

before you push to the PR branch.

There are a few reasons this technique is considered OK,

- The preferred method for PR merge is squash.
  - As long as this is the last commit, this certifies it all.
- Consistent users will likely use the .mailmap method anyways.
  - We really do prefer PGP commits, but aren't strict about this right now.
- We believe what is important is that the intent is clear and agreed upon.
  - We believe certifying a chain of commits adhere to a set of rules is equal to certifying each commit along the way.
- The DCO is additional clarification regarding the normal forms and usages of the open-source community.
  - Any contributions without certification still must meet the terms of submitting to our project.
  - By submitting patches to this project you agree to allow them to be redistributed under the project's license.
  - Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you,
    - as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
- We expect contributions through GitHub, which enforces "inbound=outbound" in their Terms of Service anyways.

### SoB Chain

Any further SoBs (Signed-off-by:'s) following the author's SoB are from

people handling and transporting the patch, but were not involved in its

development. SoB chains should reflect the **real** route a patch took

as it was propagated to the maintainers and ultimately merged, with

the first SoB entry signalling primary authorship of a single author.

### Real Identity

Also notice that a real identity is used in the `Signed-off-by` trailer.

Please don't hide your real identity, please email a maintainer if this is a concern.

### Commit Trailers

If you like, you can put extra tags at the end:

- `Co-authored-by:` is used to credit people who helped author the patch.
  - Co-authored commits will include co-authors badges on GitHub.
  - https://docs.github.com/en/pull-requests/committing-changes-to-your-project/creating-and-editing-commits/creating-a-commit-with-multiple-authors

- `on-behalf-of:` You can create commits on behalf of an organization.
  - Commits attributed to an organization include an on-behalf-of badge on GitHub.
  - https://docs.github.com/en/pull-requests/committing-changes-to-your-project/creating-and-editing-commits/creating-a-commit-on-behalf-of-an-organization

- `Reported-by:` is used to credit someone who found the bug that
the patch attempts to fix.

- `Acked-by:` says that the person who is more familiar with the area
the patch attempts to modify liked the patch.

- `Reviewed-by:`, unlike the other tags, can only be offered by the
reviewers themselves when they are completely satisfied with the
patch after a detailed analysis.

- `Tested-by:` is used to indicate that the person applied the patch
and found it to have the desired effect.

You can also create your own tag or use one that's in common usage
such as `Thanks-to:`, `Based-on-patch-by:`, or `Mentored-by:`.

Do not forget to add trailers such as `Acked-by:`, `Reviewed-by:`

and `Tested-by:` lines as necessary to credit people who helped your patch,

and `cc:` them when sending such a final version for inclusion.

A `Suggested-by:` tag indicates that the patch idea is suggested by the person
named and ensures credit to the person for the idea. Please note that this
tag should not be added without the reporter's permission, especially if the
idea was not posted in a public forum. That said, if we diligently credit our
idea reporters, they will, hopefully, be inspired to help us again in the
future.

A `Fixes:` tag indicates that the patch fixes an issue in a previous commit. It
is used to make it easy to determine where a bug originated, which can help
review a bug fix. This tag also assists the team in determining
which stable versions should receive your fix. This is the preferred
method for indicating a bug fixed by the patch.

## How to Reference

Please reference github users & teams starting with the `@` symbol.

Such as `@dezren39` or `@developing-today/CODEOWNERS`, etc.

Otherwise, if you reference a repo/pr/issue/sha, etc., we prefer the full url starting with `https://`.

For local PRs or issues, `#<number>` is an accepted alternative, though full url is preferred.

## How to Contribute

Thank you for your interest in contributing to the project!

There are many ways to contribute, and we appreciate all of them.

Opening issues, discussing design, requesting features, writing documentation,

changing the code, writing blog posts, and creating logos/artwork for projects

are all very valuable.

If you have questions, please join our discussions.

As a reminder, all participants in our community are expected to honor our code of conduct.

## Feature Proposal

To propose a change to the way that this or any other codebase works,

please open an issue directly on the relevant repository.

Include the text, `[feature-proposal]` in the title

and someone will come by and re-tag the issue appropriately.

Here's a template you can use to file a feature proposal,

though it's not necessary to use it exactly:

```
One-paragraph summary

# Details

As many details as you think are necessary to communicate how the feature
should work.

# Benefits

Benefits of the feature.

# Drawbacks

Drawbacks of the feature.
```

There is should be an example feature proposal
if you want to see what it looks like.

## Bug Reports

While bugs are unfortunate, they're a reality in software.

We can't fix what we don't know about, so please report liberally.

If you're not sure if something is a bug or not, feel free to file a bug anyway.

**If you believe reporting your bug publicly represents a security risk to
users of software developed by the project, please contact a maintainer directly
for instructions for reporting security vulnerabilities.**

If you have the chance, before reporting a bug, please search existing issues

as it's possible that someone else has already reported your error.

This doesn't always work, and sometimes it's hard to know what to search for,

so consider this extra credit. We won't mind if you accidentally file a duplicate

report, we'll typically close it and point out which issue is a duplicate of.

Opening an issue is following and filling out the fields after clicking "New Issue".

Here's a template that you can use to file a bug,

though it's not necessary to use it exactly:

```
<short summary of the bug>

I tried this:

<testcase demonstrating the bug>

I expected to see this happen: <explanation>

Instead, this happened: <explanation>
```

All three components are important:

- what you did,
- what you expected,
- what happened instead.

## Submitting code changes

To coordinate changes to our software, we use a feature of GitHub called
"pull requests".

We also call them "PRs", "contributions", or "patches".

## Commit messages

Commit messages should be self-contained and describe the motivation for a
change.

The subject line should be short, with any elaborations in the body.

If the commit closes an issue,

write `Closes #<issuenumber>` at the end of the body,

or preferably, `Closes <full url>`.

- For some guidelines, read
  - http://chris.beams.io/posts/git-commit/
- for the basics, and to become a pro.
  - https://robots.thoughtbot.com/5-useful-tips-for-a-better-commit-message

Semantic commit messages are welcome and preferred.

## Commit signing

For a variety of reasons,

- https://mikegerwitz.com/papers/git-horror-story

we prefer commit signing to verify the authenticity
and authorship of changes to our projects.

If you don't already have one, create a PGP key.

There are many ways to do this.

- https://alexcabal.com/creating-the-perfect-gpg-keypair
  - has a decent explanation of how to do it
  - and some of the tradeoffs involved.

This key doesn't need to be used for other purposes,

although you should consider using it for emails to the mailing list or maintainers!

Once you have a keypair and it is uploaded to the keyservers,

- https://mikegerwitz.com/papers/git-horror-story#trust-ensure

Please sign all commits you intend to include in a pull request.

## Forking the repositories.

We use a custom system for managing our large amount of repositories.

We have one main "code" repository,

which contains scripts for fetching the other repositories as needed.

To fork a repository,

navigate to it on GitHub and click the "Fork" button,

which is to the right on the top of the UI.

Then, you can interact with that repository using the `origin` remote.

## Recognize the Developer Certificate of Origin

Before we can accept code you have written into our repositories,

you must agree to the Developer Certificate of Origin, Version 1.1.

By committing to this project, you agree to the [Developer Certificate of Origin, Version 1.1](https://developercertificate.org/)

([`DCO-1.1`](DCO-1.1) in the root directory of this source tree.)

Please ensure to certify that your patch set adheres to these rules,

and has been "Signed-off-by" according to the guidelines

in [`COPYRIGHT.md`](COPYRIGHT.md) in the root directory of this source tree.

Please only "Signed-off-by" if, and only if, you can certify to the terms of the DCO.

It is quite minimalist, and is also used by the Linux kernel.

One way to "Signed-off-by" is to use `git commit -s` when committing your changes.

Another way to accept and recognize the [`DCO-1.1`](DCO-1.1) for all commits,

is to add yourself to the [`.mailmap`](.mailmap) in the root directory of this source tree,

with the names you'd like to be called by and

all of the email addresses you'll use (you can add to this later),

as well as your PGP key fingerprint in a comment.

Again, please only "Signed-off-by" if, and only if, you can certify to the terms of the DCO.

Create a new commit with those changes and send a pull request.

## Submitting a pull request

To submit a pull request,

push the commits you want to include to a branch on your fork of the relevant repository.

Refer to the documentation to see how to proceed from there.

- https://docs.github.com/en/desktop/contributing-and-collaborating-using-github-desktop/working-with-your-remote-repository-on-github-or-github-enterprise/creating-an-issue-or-pull-request
- https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/creating-a-pull-request

Please open pull requests against the `main` branch.

If needed, we will merge into a separate branch manually.

All pull requests are reviewed by another person.

If you want to request that a specific person reviews your pull request,

you can specify the "assignee" when you create the pull request.

## Writing Documentation

Documentation improvements are very welcome.

Documentation pull requests function in the same way as other pull requests.
