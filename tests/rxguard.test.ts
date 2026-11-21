// RxGuard demo tests for Solana Playground.
// No imports needed: web3, anchor, pg, BN and assert are globals in Playground.

// Build the on-chain fingerprint: SHA-256(prescription JSON + random salt).
// Only these 32 bytes go on-chain. The JSON (patient, drug) never leaves the client.
async function rxHash(drug: string): Promise<Buffer> {
  const salt = web3.Keypair.generate().publicKey.toBase58(); // fresh random salt each run
  const rx = JSON.stringify({ patientRef: "demo-123", drug, qty: 20, salt });
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(rx));
  return Buffer.from(new Uint8Array(digest));
}

function rxPda(hash: Buffer): web3.PublicKey {
  return web3.PublicKey.findProgramAddressSync([Buffer.from("rx"), hash], pg.PROGRAM_ID)[0];
}

// Run a tx we EXPECT to fail and return the Anchor error code (or the raw message).
async function expectFail(p: Promise<string>): Promise<string> {
  try {
    await p;
  } catch (err: any) {
    const code = err?.error?.errorCode?.code ?? err?.message ?? String(err);
    console.log(`   ⛔ Rejected on-chain: ${err?.error?.errorMessage ?? code}`);
    return code;
  }
  throw new Error("Transaction was expected to fail but succeeded!");
}

describe("RxGuard", () => {
  const doctor = pg.wallet.publicKey; // pg.wallet pays all fees
  const pharmacyA = web3.Keypair.generate(); // authorized pharmacy (no SOL needed, only signs)
  const pharmacyB = web3.Keypair.generate(); // rogue pharmacy
  let hash: Buffer;
  let pda: web3.PublicKey;

  const fill = (rx: web3.PublicKey, pharmacy: web3.Keypair) =>
    pg.program.methods
      .fillPrescription()
      .accounts({ prescription: rx, pharmacy: pharmacy.publicKey })
      .signers([pharmacy])
      .rpc();

  const issue = (h: Buffer, rx: web3.PublicKey) =>
    pg.program.methods
      .issuePrescription(Array.from(h), pharmacyA.publicKey)
      .accounts({ prescription: rx, doctor, systemProgram: web3.SystemProgram.programId })
      .rpc();

  before(async () => {
    hash = await rxHash("oxycodone 5mg");
    pda = rxPda(hash);
  });

  it("1. ✅ Doctor issues the prescription", async () => {
    console.log("🩺 Doctor issues an Rx to Pharmacy A. Only its hash goes on-chain.");
    const tx = await issue(hash, pda);
    await pg.connection.confirmTransaction(tx);
    const rx = await pg.program.account.prescription.fetch(pda);
    assert("issued" in rx.status);
    console.log(`   Prescription PDA: ${pda.toBase58()}`);
    console.log(`   Explorer: https://explorer.solana.com/address/${pda.toBase58()}?cluster=devnet`);
  });

  it("2. ❌ Rogue Pharmacy B tries to fill it", async () => {
    console.log("🏴 Pharmacy B (not the assigned pharmacy) tries to fill...");
    const code = await expectFail(fill(pda, pharmacyB));
    assert.equal(code, "UnauthorizedPharmacy");
  });

  it("3. ✅ Pharmacy A fills the prescription", async () => {
    console.log("💊 Pharmacy A fills the Rx...");
    const tx = await fill(pda, pharmacyA);
    await pg.connection.confirmTransaction(tx);
    const rx = await pg.program.account.prescription.fetch(pda);
    assert("filled" in rx.status);
    assert(rx.filledAt.toNumber() > 0);
    console.log(`   Filled at unix time ${rx.filledAt.toString()}`);
  });

  it("4. ❌ Pharmacy A tries to fill it AGAIN (double-fill)", async () => {
    console.log("🔁 Pharmacy A tries to fill the same Rx a second time...");
    const code = await expectFail(fill(pda, pharmacyA));
    assert.equal(code, "AlreadyFilled");
    console.log("   🛡️ Double-fill blocked!");
  });

  it("5. ❌ Doctor tries to issue the SAME prescription again", async () => {
    console.log("📄 Re-issuing the exact same prescription hash...");
    const code = await expectFail(issue(hash, pda));
    // Not an Anchor error: the System Program refuses because the PDA already exists.
    assert(code.length > 0);
    console.log("   The PDA already exists, so the same Rx can't be issued twice.");
  });

  it("6. ✅ Bonus: cancelled prescriptions can't be filled", async () => {
    console.log("✂️ Doctor issues a second Rx, then cancels it...");
    const hash2 = await rxHash("hydrocodone 10mg");
    const pda2 = rxPda(hash2);
    await pg.connection.confirmTransaction(await issue(hash2, pda2));
    const tx = await pg.program.methods
      .cancelPrescription()
      .accounts({ prescription: pda2, doctor })
      .rpc();
    await pg.connection.confirmTransaction(tx);
    const rx = await pg.program.account.prescription.fetch(pda2);
    assert("cancelled" in rx.status);
    console.log("   Pharmacy A tries to fill the cancelled Rx...");
    const code = await expectFail(fill(pda2, pharmacyA));
    assert.equal(code, "PrescriptionCancelled");
  });
});
