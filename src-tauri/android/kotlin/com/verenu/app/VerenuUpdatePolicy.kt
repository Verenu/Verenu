package com.verenu.app

internal object VerenuUpdatePolicy {
  fun accepts(
    installedPackage: String,
    candidatePackage: String,
    installedVersion: Long,
    candidateVersion: Long,
    installedSigners: Set<String>,
    candidateSigners: Set<String>,
  ): Boolean = candidatePackage == installedPackage &&
    candidateVersion >= installedVersion && candidateSigners.isNotEmpty() &&
    candidateSigners == installedSigners
}
