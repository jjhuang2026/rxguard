// RxGuard: prescription double-fill prevention on Solana.
//
// PRIVACY RULE: no patient data or drug names ever go on-chain.
// We store only a 32-byte SHA-256 hash of the off-chain prescription
// (JSON + a random salt, so nobody can brute-force the hash back to a drug/patient).
// All HIPAA-relevant data stays off-chain with the doctor and pharmacy.
use anchor_lang::prelude::*;

// Solana Playground fills this in automatically when you click Build.
declare_id!("11111111111111111111111111111111");

#[program]
pub mod rxguard {
    use super::*;

    /// Doctor writes the prescription fingerprint on-chain and names the one pharmacy allowed to fill it.
    pub fn issue_prescription(
        ctx: Context<IssuePrescription>,
        rx_hash: [u8; 32],
        pharmacy: Pubkey,
    ) -> Result<()> {
        let rx = &mut ctx.accounts.prescription;
        rx.doctor = ctx.accounts.doctor.key();
        rx.pharmacy = pharmacy;
        rx.rx_hash = rx_hash;
        rx.status = RxStatus::Issued;
        rx.issued_at = Clock::get()?.unix_timestamp;
        rx.filled_at = 0;
        rx.bump = ctx.bumps.prescription;

        emit!(PrescriptionIssued { rx_hash, doctor: rx.doctor, pharmacy });
        Ok(())
    }

    /// The assigned pharmacy marks the prescription as filled. This works exactly once.
    pub fn fill_prescription(ctx: Context<FillPrescription>) -> Result<()> {
        let rx = &mut ctx.accounts.prescription;
        // Status gate: only an Issued prescription can be filled.
        match rx.status {
            RxStatus::Issued => {}
            RxStatus::Filled => return err!(RxError::AlreadyFilled),
            RxStatus::Cancelled => return err!(RxError::PrescriptionCancelled),
        }
        rx.status = RxStatus::Filled;
        rx.filled_at = Clock::get()?.unix_timestamp;

        emit!(PrescriptionFilled {
            rx_hash: rx.rx_hash,
            pharmacy: rx.pharmacy,
            filled_at: rx.filled_at,
        });
        Ok(())
    }

    /// The issuing doctor can cancel a prescription that hasn't been filled yet.
    pub fn cancel_prescription(ctx: Context<CancelPrescription>) -> Result<()> {
        let rx = &mut ctx.accounts.prescription;
        match rx.status {
            RxStatus::Issued => {}
            RxStatus::Filled => return err!(RxError::AlreadyFilled),
            RxStatus::Cancelled => return err!(RxError::PrescriptionCancelled),
        }
        rx.status = RxStatus::Cancelled;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(rx_hash: [u8; 32])]
pub struct IssuePrescription<'info> {
    // PDA = an account whose address is derived from seeds + this program's ID.
    // Seeding on the hash means one prescription maps to exactly one address,
    // so `init` fails if anyone tries to issue the same prescription twice.
    #[account(
        init,
        payer = doctor,
        space = 8 + Prescription::INIT_SPACE, // 8 bytes = Anchor's account discriminator
        seeds = [b"rx", rx_hash.as_ref()],
        bump
    )]
    pub prescription: Account<'info, Prescription>,
    // The doctor signs and pays rent for the new account.
    #[account(mut)]
    pub doctor: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FillPrescription<'info> {
    // Re-derive the PDA with the stored bump so a fake account can't be swapped in.
    // The constraint rejects any pharmacy other than the one the doctor chose.
    #[account(
        mut,
        seeds = [b"rx", prescription.rx_hash.as_ref()],
        bump = prescription.bump,
        constraint = prescription.pharmacy == pharmacy.key() @ RxError::UnauthorizedPharmacy
    )]
    pub prescription: Account<'info, Prescription>,
    // The pharmacy must sign. It pays nothing (the fee payer can be someone else).
    pub pharmacy: Signer<'info>,
}

#[derive(Accounts)]
pub struct CancelPrescription<'info> {
    // `has_one = doctor` checks prescription.doctor == doctor.key().
    #[account(
        mut,
        seeds = [b"rx", prescription.rx_hash.as_ref()],
        bump = prescription.bump,
        has_one = doctor @ RxError::UnauthorizedDoctor
    )]
    pub prescription: Account<'info, Prescription>,
    pub doctor: Signer<'info>,
}

#[account]
#[derive(InitSpace)]
pub struct Prescription {
    pub doctor: Pubkey,     // who issued it
    pub pharmacy: Pubkey,   // the only pharmacy allowed to fill it
    pub rx_hash: [u8; 32],  // salted SHA-256 of the off-chain prescription
    pub status: RxStatus,
    pub issued_at: i64,
    pub filled_at: i64,     // 0 until filled
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum RxStatus {
    Issued,
    Filled,
    Cancelled,
}

#[event]
pub struct PrescriptionIssued {
    pub rx_hash: [u8; 32],
    pub doctor: Pubkey,
    pub pharmacy: Pubkey,
}

#[event]
pub struct PrescriptionFilled {
    pub rx_hash: [u8; 32],
    pub pharmacy: Pubkey,
    pub filled_at: i64,
}

#[error_code]
pub enum RxError {
    #[msg("This prescription has already been filled.")]
    AlreadyFilled,
    #[msg("This pharmacy is not authorized to fill this prescription.")]
    UnauthorizedPharmacy,
    #[msg("Only the issuing doctor can cancel this prescription.")]
    UnauthorizedDoctor,
    #[msg("This prescription was cancelled by the doctor.")]
    PrescriptionCancelled,
}
