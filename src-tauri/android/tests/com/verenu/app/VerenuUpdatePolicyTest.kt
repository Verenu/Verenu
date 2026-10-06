package com.verenu.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuUpdatePolicyTest {
  @Test fun acceptsUpgradeAndSameVersionReinstall() {
    assertTrue(accepts(version = 101))
    assertTrue(accepts(version = 100))
  }

  @Test fun blocksDowngradesOtherAppsAndUnsignedOrDifferentSigners() {
    assertFalse(accepts(version = 99))
    assertFalse(accepts(name = "com.example.other"))
    assertFalse(accepts(signers = emptySet()))
    assertFalse(accepts(signers = setOf("development-key")))
    assertFalse(accepts(signers = setOf("release-key", "extra-key")))
  }

  private fun accepts(version: Long = 101, name: String = "com.verenu.app", signers: Set<String> = setOf("release-key")) =
    VerenuUpdatePolicy.accepts("com.verenu.app", name, 100, version, setOf("release-key"), signers)
}
