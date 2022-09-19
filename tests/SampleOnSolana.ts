import * as anchor from "@project-serum/anchor";
import { Program } from "@project-serum/anchor";
import assert from "assert";
import * as spl from '@solana/spl-token';
import { SampleOnSolana } from "../target/types/sample_on_solana";


interface PDAParameters {
    escrowWalletKey: anchor.web3.PublicKey,
    stateKey: anchor.web3.PublicKey,
    escrowBump: number,
    stateBump: number,
    idx: anchor.BN,
}

describe("SampleOnSolana", () => {
  // Configure the client to use the local cluster.
  
//const provider = anchor.Provider.env();
  const provider = anchor.Provider.local();
  anchor.setProvider(provider);

  const program = anchor.workspace.SampleOnSolana;

  let mintAddress: anchor.web3.PublicKey;
  let consumer: anchor.web3.Keypair;
  let consumerWallet: anchor.web3.PublicKey;
  let supplier: anchor.web3.Keypair;

  let pda: PDAParameters;

  const getPdaParams = async (connection: anchor.web3.Connection, consumer: anchor.web3.PublicKey, supplier: anchor.web3.PublicKey, mint: anchor.web3.PublicKey): Promise<PDAParameters> => {
      const uid = new anchor.BN(parseInt((Date.now() / 1000).toString()));
      const uidBuffer = uid.toBuffer('le', 8);

      let [statePubKey, stateBump] = await anchor.web3.PublicKey.findProgramAddress(
          [Buffer.from("state"), consumer.toBuffer(), supplier.toBuffer(), mint.toBuffer(), uidBuffer], program.programId,
      );
      let [walletPubKey, walletBump] = await anchor.web3.PublicKey.findProgramAddress(
          [Buffer.from("wallet"), consumer.toBuffer(), supplier.toBuffer(), mint.toBuffer(), uidBuffer], program.programId,
      );
      return {
          idx: uid,
          escrowBump: walletBump,
          escrowWalletKey: walletPubKey,
          stateBump,
          stateKey: statePubKey,
      }
  }

  const createMint = async (connection: anchor.web3.Connection): Promise<anchor.web3.PublicKey> => {
      const tokenMint = new anchor.web3.Keypair();
      const lamportsForMint = await provider.connection.getMinimumBalanceForRentExemption(spl.MintLayout.span);
      let tx = new anchor.web3.Transaction();

      // Allocate mint
      tx.add(
          anchor.web3.SystemProgram.createAccount({
              programId: spl.TOKEN_PROGRAM_ID,
              space: spl.MintLayout.span,
              fromPubkey: provider.wallet.publicKey,
              newAccountPubkey: tokenMint.publicKey,
              lamports: lamportsForMint,
          })
      )
      // Allocate wallet account
      tx.add(
          spl.Token.createInitMintInstruction(
              spl.TOKEN_PROGRAM_ID,
              tokenMint.publicKey,
              6,
              provider.wallet.publicKey,
              provider.wallet.publicKey,
          )
      );
      const signature = await provider.send(tx, [tokenMint]);

      console.log(`[${tokenMint.publicKey}] Created new mint account at ${signature}`);
      return tokenMint.publicKey;
  }

  const createUserAndAssociatedWallet = async (connection: anchor.web3.Connection, mint?: anchor.web3.PublicKey): Promise<[anchor.web3.Keypair, anchor.web3.PublicKey | undefined]> => {
      const user = new anchor.web3.Keypair();
      let userAssociatedTokenAccount: anchor.web3.PublicKey | undefined = undefined;

      // Fund user with some SOL
      let txFund = new anchor.web3.Transaction();
      txFund.add(anchor.web3.SystemProgram.transfer({
          fromPubkey: provider.wallet.publicKey,
          toPubkey: user.publicKey,
          lamports: 5 * anchor.web3.LAMPORTS_PER_SOL,
      }));
      const sigTxFund = await provider.send(txFund);
      console.log(`[${user.publicKey.toBase58()}] Funded new account with 5 SOL: ${sigTxFund}`);

      if (mint) {
          // Create a token account for the user and mint some tokens
          userAssociatedTokenAccount = await spl.Token.getAssociatedTokenAddress(
              spl.ASSOCIATED_TOKEN_PROGRAM_ID,
              spl.TOKEN_PROGRAM_ID,
              mint,
              user.publicKey
          )

          const txFundTokenAccount = new anchor.web3.Transaction();
          txFundTokenAccount.add(spl.Token.createAssociatedTokenAccountInstruction(
              spl.ASSOCIATED_TOKEN_PROGRAM_ID,
              spl.TOKEN_PROGRAM_ID,
              mint,
              userAssociatedTokenAccount,
              user.publicKey,
              user.publicKey,
          ))
          txFundTokenAccount.add(spl.Token.createMintToInstruction(
              spl.TOKEN_PROGRAM_ID,
              mint,
              userAssociatedTokenAccount,
              provider.wallet.publicKey,
              [],
              1337000000,
          ));
          const txFundTokenSig = await provider.send(txFundTokenAccount, [user]);
          console.log(`[${userAssociatedTokenAccount.toBase58()}] New associated account for mint ${mint.toBase58()}: ${txFundTokenSig}`);
      }
      return [user, userAssociatedTokenAccount];
  }

  const readAccount = async (accountPublicKey: anchor.web3.PublicKey, provider: anchor.Provider): Promise<[spl.AccountInfo, string]> => {
      const tokenInfoLol = await provider.connection.getAccountInfo(accountPublicKey);
      const data = Buffer.from(tokenInfoLol.data);
      const accountInfo: spl.AccountInfo = spl.AccountLayout.decode(data);

      const amount = (accountInfo.amount as any as Buffer).readBigUInt64LE();
      return [accountInfo, amount.toString()];
  }

  const readMint = async (mintPublicKey: anchor.web3.PublicKey, provider: anchor.Provider): Promise<spl.MintInfo> => {
      const tokenInfo = await provider.connection.getAccountInfo(mintPublicKey);
      const data = Buffer.from(tokenInfo.data);
      const accountInfo = spl.MintLayout.decode(data);
      return {
          ...accountInfo,
          mintAuthority: accountInfo.mintAuthority == null ? null : anchor.web3.PublicKey.decode(accountInfo.mintAuthority),
          freezeAuthority: accountInfo.freezeAuthority == null ? null : anchor.web3.PublicKey.decode(accountInfo.freezeAuthority),
      }
  }

  beforeEach(async () => {
      mintAddress = await createMint(provider.connection);
      [consumer, consumerWallet] = await createUserAndAssociatedWallet(provider.connection, mintAddress);

      let _rest;
      [supplier, ..._rest] = await createUserAndAssociatedWallet(provider.connection);

      pda = await getPdaParams(provider.connection, consumer.publicKey, supplier.publicKey, mintAddress);
  });

  it('can initialize a payment by Consumer', async () => {
      const [, consumerBalancePre] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePre, '1337000000');

      const amount = new anchor.BN(20000000);

      // Initialize mint account and fund the account
      const tx1 = await program.rpc.initializeNewGrant(pda.idx, pda.stateBump, pda.escrowBump, amount, {
          accounts: {
              applicationState: pda.stateKey,
              escrowWalletState: pda.escrowWalletKey,
              mintOfTokenBeingSent: mintAddress,
              userSending: consumer.publicKey,
              userReceiving: supplier.publicKey,
              walletToWithdrawFrom: consumerWallet,

              systemProgram: anchor.web3.SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
              tokenProgram: spl.TOKEN_PROGRAM_ID,
          },
          signers: [consumer],
      });
      console.log(`Initialized a new Kal instance. Consumer will pay supplier 20 tokens`);

      // Assert that 20 tokens were moved from Consumer's account to the escrow.
      const [, consumerBalancePost] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePost, '1317000000');
      const [, escrowBalancePost] = await readAccount(pda.escrowWalletKey, provider);
      assert.equal(escrowBalancePost, '20000000');

      const state = await program.account.state.fetch(pda.stateKey);
      assert.equal(state.amountTokens.toString(), '20000000');
      assert.equal(state.stage.toString(), '1');
  })

  it('can send escrow funds to Supplier', async () => {
      const [, consumerBalancePre] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePre, '1337000000');

      const amount = new anchor.BN(20000000);

      // Initialize mint account and fund the account
      const tx1 = await program.rpc.initializeNewGrant(pda.idx, pda.stateBump, pda.escrowBump, amount, {
          accounts: {
              applicationState: pda.stateKey,
              escrowWalletState: pda.escrowWalletKey,
              mintOfTokenBeingSent: mintAddress,
              userSending: consumer.publicKey,
              userReceiving: supplier.publicKey,
              walletToWithdrawFrom: consumerWallet,

              systemProgram: anchor.web3.SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
              tokenProgram: spl.TOKEN_PROGRAM_ID,
          },
          signers: [consumer],
      });
      console.log(`Initialized a new Kal instance. Consumer will pay supplier 20 tokens`);

      // Assert that 20 tokens were moved from Consumer's account to the escrow.
      const [, consumerBalancePost] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePost, '1317000000');
      const [, escrowBalancePost] = await readAccount(pda.escrowWalletKey, provider);
      assert.equal(escrowBalancePost, '20000000');

      // Create a token account for Supplier.
      const supplierTokenAccount = await spl.Token.getAssociatedTokenAddress(
          spl.ASSOCIATED_TOKEN_PROGRAM_ID,
          spl.TOKEN_PROGRAM_ID,
          mintAddress,
          supplier.publicKey
      )
      const tx2 = await program.rpc.completeGrant(pda.idx, pda.stateBump, pda.escrowBump, {
          accounts: {
              applicationState: pda.stateKey,
              escrowWalletState: pda.escrowWalletKey,
              mintOfTokenBeingSent: mintAddress,
              userSending: consumer.publicKey,
              userReceiving: supplier.publicKey,
              walletToDepositTo: supplierTokenAccount,

              systemProgram: anchor.web3.SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
              tokenProgram: spl.TOKEN_PROGRAM_ID,
              associatedTokenProgram: spl.ASSOCIATED_TOKEN_PROGRAM_ID,
          },
          signers: [supplier],
      });

      // Assert that 20 tokens were sent back.
      const [, supplierBalance] = await readAccount(supplierTokenAccount, provider);
      assert.equal(supplierBalance, '20000000');

      // // Assert that escrow was correctly closed.
      try {
          await readAccount(pda.escrowWalletKey, provider);
          return assert.fail("Account should be closed");
      } catch (e) {
          assert.equal(e.message, "Cannot read properties of null (reading 'data')");
      }
  })

  it('can pull back funds once they are deposited', async () => {
      const [, consumerBalancePre] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePre, '1337000000');

      const amount = new anchor.BN(20000000);

      // Initialize mint account and fund the account
      const tx1 = await program.rpc.initializeNewGrant(pda.idx, pda.stateBump, pda.escrowBump, amount, {
          accounts: {
              applicationState: pda.stateKey,
              escrowWalletState: pda.escrowWalletKey,
              mintOfTokenBeingSent: mintAddress,
              userSending: consumer.publicKey,
              userReceiving: supplier.publicKey,
              walletToWithdrawFrom: consumerWallet,

              systemProgram: anchor.web3.SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
              tokenProgram: spl.TOKEN_PROGRAM_ID,
          },
          signers: [consumer],
      });
      console.log(`Initialized a new Kal instance. Consumer will pay supplier 20 tokens`);

      // Assert that 20 tokens were moved from Consumer's account to the escrow.
      const [, consumerBalancePost] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalancePost, '1317000000');
      const [, escrowBalancePost] = await readAccount(pda.escrowWalletKey, provider);
      assert.equal(escrowBalancePost, '20000000');

      // Withdraw the funds back
      const tx2 = await program.rpc.pullBack(pda.idx, pda.stateBump, pda.escrowBump, {
          accounts: {
              applicationState: pda.stateKey,
              escrowWalletState: pda.escrowWalletKey,
              mintOfTokenBeingSent: mintAddress,
              userSending: consumer.publicKey,
              userReceiving: supplier.publicKey,
              refundWallet: consumerWallet,

              systemProgram: anchor.web3.SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
              tokenProgram: spl.TOKEN_PROGRAM_ID,
          },
          signers: [consumer],
      });

      // Assert that 20 tokens were sent back.
      const [, consumerBalanceRefund] = await readAccount(consumerWallet, provider);
      assert.equal(consumerBalanceRefund, '1337000000');

      // Assert that escrow was correctly closed.
      try {
          await readAccount(pda.escrowWalletKey, provider);
          return assert.fail("Account should be closed");
      } catch (e) {
          assert.equal(e.message, "Cannot read properties of null (reading 'data')");
      }

      const state = await program.account.state.fetch(pda.stateKey);
      assert.equal(state.amountTokens.toString(), '20000000');
      assert.equal(state.stage.toString(), '3');

  })
//   const provider = anchor.AnchorProvider.env();
//   anchor.setProvider(provider);

//   const program = anchor.workspace.SampleOnSolana as Program<SampleOnSolana>;

//   const counterAccount = anchor.web3.Keypair.generate();

//   it("Is initialized!", async () => {
//     // Add your test here.
//     await program.rpc.create({
//       accounts: {
//           counterAccount: counterAccount.publicKey,
//           user: provider.wallet.publicKey,
//           systemProgram: anchor.web3.SystemProgram.programId,
//       },
//       signers: [counterAccount]
//   } as any)
//   });

//   it("Increment counter", async () => {
//     await program.rpc.increment({
//         accounts: {
//             counterAccount: counterAccount.publicKey
//         }
//     } as any)
// })

// it("Fetch account", async () => {
//     const account: any = await
//     program.account.counterAccount.fetch(counterAccount.publicKey)
//     console.log(account.count)
// })
});
