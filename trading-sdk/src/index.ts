import { Buffer } from "buffer";
import { Address } from '@stellar/stellar-sdk';
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from '@stellar/stellar-sdk/contract';
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Typepoint,
  Duration,
} from '@stellar/stellar-sdk/contract';
export * from '@stellar/stellar-sdk'
export * as contract from '@stellar/stellar-sdk/contract'
export * as rpc from '@stellar/stellar-sdk/rpc'

if (typeof window !== 'undefined') {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}




export const TradingError = {
  10: {message:"StalePrice"},
  11: {message:"NoPrice"},
  66: {message:"NotUnlocked"},
  67: {message:"InvalidConfig"},
  68: {message:"MaxPositions"},
  69: {message:"InvalidAction"},
  20: {message:"BadRequest"}
}

export type TradingDataKey = {tag: "MarketConfig", values: readonly [Asset]} | {tag: "MarketInit", values: readonly [Asset]} | {tag: "MarketData", values: readonly [Asset]} | {tag: "UserPositions", values: readonly [string]} | {tag: "Position", values: readonly [u32]};


export interface Market {
  asset: Asset;
  config: MarketConfig;
  data: MarketData;
}


export interface Request {
  action: RequestType;
  data: Option<i128>;
  position: u32;
}

/**
 * The type of request to be made against the pool
 */
export enum RequestType {
  Close = 0,
  Fill = 1,
  StopLoss = 2,
  TakeProfit = 3,
  Liquidation = 4,
  Cancel = 5,
  DepositCollateral = 6,
  WithdrawCollateral = 7,
  SetTakeProfit = 8,
  SetStopLoss = 9,
}


export interface SubmitResult {
  results: Array<u32>;
  transfers: Map<string, i128>;
}


export interface TradingConfig {
  caller_take_rate: i128;
  max_positions: u32;
  oracle: string;
}

/**
 * Position status
 */
export type PositionStatus = {tag: "Pending", values: void} | {tag: "Open", values: void} | {tag: "Closed", values: void};


export interface MarketConfig {
  base_fee: i128;
  enabled: boolean;
  init_margin: i128;
  maintenance_margin: i128;
  max_collateral: i128;
  max_hourly_rate: i128;
  max_payout: i128;
  min_collateral: i128;
  min_hourly_rate: i128;
  price_impact_scalar: i128;
  target_hourly_rate: i128;
  target_utilization: i128;
  total_available: i128;
}


export interface QueuedMarketInit {
  config: MarketConfig;
  unlock_time: u64;
}


export interface MarketData {
  last_update: u64;
  long_collateral: i128;
  long_count: u32;
  long_interest_index: i128;
  long_notional_size: i128;
  short_collateral: i128;
  short_count: u32;
  short_interest_index: i128;
  short_notional_size: i128;
}


/**
 * Structure to store information about a position
 */
export interface Position {
  asset: Asset;
  close_price: i128;
  collateral: i128;
  created_at: u64;
  entry_price: i128;
  id: u32;
  interest_index: i128;
  is_long: boolean;
  notional_size: i128;
  status: PositionStatus;
  stop_loss: i128;
  take_profit: i128;
  user: string;
}


/**
 * Price data for an asset at a specific timestamp
 */
export interface PriceData {
  price: i128;
  timestamp: u64;
}

/**
 * Asset type
 */
export type Asset = {tag: "Stellar", values: readonly [string]} | {tag: "Other", values: readonly [string]};


/**
 * Storage key for enumeration of accounts per role.
 */
export interface RoleAccountKey {
  index: u32;
  role: string;
}

/**
 * Storage keys for the data associated with the access control
 */
export type AccessControlStorageKey = {tag: "RoleAccounts", values: readonly [RoleAccountKey]} | {tag: "HasRole", values: readonly [string, string]} | {tag: "RoleAccountsCount", values: readonly [string]} | {tag: "RoleAdmin", values: readonly [string]} | {tag: "Admin", values: void} | {tag: "PendingAdmin", values: void};

export const AccessControlError = {
  1210: {message:"Unauthorized"},
  1211: {message:"AdminNotSet"},
  1212: {message:"IndexOutOfBounds"},
  1213: {message:"AdminRoleNotFound"},
  1214: {message:"RoleCountIsNotZero"},
  1215: {message:"RoleNotFound"},
  1216: {message:"AdminAlreadySet"},
  1217: {message:"RoleNotHeld"},
  1218: {message:"RoleIsEmpty"}
}

/**
 * Storage keys for `Ownable` utility.
 */
export type OwnableStorageKey = {tag: "Owner", values: void} | {tag: "PendingOwner", values: void};

export const OwnableError = {
  1220: {message:"OwnerNotSet"},
  1221: {message:"TransferInProgress"},
  1222: {message:"OwnerAlreadySet"}
}

export const RoleTransferError = {
  1200: {message:"NoPendingTransfer"},
  1201: {message:"InvalidLiveUntilLedger"},
  1202: {message:"InvalidPendingAccount"}
}

export interface Client {
  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  initialize: ({name, vault, config}: {name: string, vault: string, config: TradingConfig}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_config transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_config: ({config}: {config: TradingConfig}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a queue_set_market transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  queue_set_market: ({asset, config}: {asset: Asset, config: MarketConfig}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a cancel_set_market transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  cancel_set_market: ({asset}: {asset: Asset}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_market transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_market: ({asset}: {asset: Asset}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a set_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  set_status: ({status}: {status: u32}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a create_position transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  create_position: ({user, asset, collateral, notional_size, is_long, entry_price}: {user: string, asset: Asset, collateral: i128, notional_size: i128, is_long: boolean, entry_price: i128}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a submit transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  submit: ({caller, requests}: {caller: string, requests: Array<Request>}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<SubmitResult>>

  /**
   * Construct and simulate a upgrade transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  upgrade: ({wasm_hash}: {wasm_hash: Buffer}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a get_owner transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_owner: (options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<Option<string>>>

  /**
   * Construct and simulate a transfer_ownership transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  transfer_ownership: ({new_owner, live_until_ledger}: {new_owner: string, live_until_ledger: u32}, options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a accept_ownership transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  accept_ownership: (options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

  /**
   * Construct and simulate a renounce_ownership transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  renounce_ownership: (options?: {
    /**
     * The fee to pay for the transaction. Default: BASE_FEE
     */
    fee?: number;

    /**
     * The maximum amount of time to wait for the transaction to complete. Default: DEFAULT_TIMEOUT
     */
    timeoutInSeconds?: number;

    /**
     * Whether to automatically simulate the transaction when constructing the AssembledTransaction. Default: true
     */
    simulate?: boolean;
  }) => Promise<AssembledTransaction<null>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
        /** Constructor/Initialization Args for the contract's `__constructor` method */
        {owner}: {owner: string},
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy({owner}, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAADFRyYWRpbmdFcnJvcgAAAAcAAAAAAAAAClN0YWxlUHJpY2UAAAAAAAoAAAAAAAAAB05vUHJpY2UAAAAACwAAAAAAAAALTm90VW5sb2NrZWQAAAAAQgAAAAAAAAANSW52YWxpZENvbmZpZwAAAAAAAEMAAAAAAAAADE1heFBvc2l0aW9ucwAAAEQAAAAAAAAADUludmFsaWRBY3Rpb24AAAAAAABFAAAAAAAAAApCYWRSZXF1ZXN0AAAAAAAU",
        "AAAAAgAAAAAAAAAAAAAADlRyYWRpbmdEYXRhS2V5AAAAAAAFAAAAAQAAAAAAAAAMTWFya2V0Q29uZmlnAAAAAQAAB9AAAAAFQXNzZXQAAAAAAAABAAAAAAAAAApNYXJrZXRJbml0AAAAAAABAAAH0AAAAAVBc3NldAAAAAAAAAEAAAAAAAAACk1hcmtldERhdGEAAAAAAAEAAAfQAAAABUFzc2V0AAAAAAAAAQAAAAAAAAANVXNlclBvc2l0aW9ucwAAAAAAAAEAAAATAAAAAQAAAAAAAAAIUG9zaXRpb24AAAABAAAABA==",
        "AAAAAAAAAAAAAAANX19jb25zdHJ1Y3RvcgAAAAAAAAEAAAAAAAAABW93bmVyAAAAAAAAEwAAAAA=",
        "AAAAAAAAAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAwAAAAAAAAAEbmFtZQAAABAAAAAAAAAABXZhdWx0AAAAAAAAEwAAAAAAAAAGY29uZmlnAAAAAAfQAAAADVRyYWRpbmdDb25maWcAAAAAAAAA",
        "AAAAAAAAAAAAAAAKc2V0X2NvbmZpZwAAAAAAAQAAAAAAAAAGY29uZmlnAAAAAAfQAAAADVRyYWRpbmdDb25maWcAAAAAAAAA",
        "AAAAAAAAAAAAAAAQcXVldWVfc2V0X21hcmtldAAAAAIAAAAAAAAABWFzc2V0AAAAAAAH0AAAAAVBc3NldAAAAAAAAAAAAAAGY29uZmlnAAAAAAfQAAAADE1hcmtldENvbmZpZwAAAAA=",
        "AAAAAAAAAAAAAAARY2FuY2VsX3NldF9tYXJrZXQAAAAAAAABAAAAAAAAAAVhc3NldAAAAAAAB9AAAAAFQXNzZXQAAAAAAAAA",
        "AAAAAAAAAAAAAAAKc2V0X21hcmtldAAAAAAAAQAAAAAAAAAFYXNzZXQAAAAAAAfQAAAABUFzc2V0AAAAAAAAAA==",
        "AAAAAAAAAAAAAAAKc2V0X3N0YXR1cwAAAAAAAQAAAAAAAAAGc3RhdHVzAAAAAAAEAAAAAA==",
        "AAAAAAAAAAAAAAAPY3JlYXRlX3Bvc2l0aW9uAAAAAAYAAAAAAAAABHVzZXIAAAATAAAAAAAAAAVhc3NldAAAAAAAB9AAAAAFQXNzZXQAAAAAAAAAAAAACmNvbGxhdGVyYWwAAAAAAAsAAAAAAAAADW5vdGlvbmFsX3NpemUAAAAAAAALAAAAAAAAAAdpc19sb25nAAAAAAEAAAAAAAAAC2VudHJ5X3ByaWNlAAAAAAsAAAABAAAABA==",
        "AAAAAAAAAAAAAAAGc3VibWl0AAAAAAACAAAAAAAAAAZjYWxsZXIAAAAAABMAAAAAAAAACHJlcXVlc3RzAAAD6gAAB9AAAAAHUmVxdWVzdAAAAAABAAAH0AAAAAxTdWJtaXRSZXN1bHQ=",
        "AAAAAAAAAAAAAAAHdXBncmFkZQAAAAABAAAAAAAAAAl3YXNtX2hhc2gAAAAAAAPuAAAAIAAAAAA=",
        "AAAAAAAAAAAAAAAJZ2V0X293bmVyAAAAAAAAAAAAAAEAAAPoAAAAEw==",
        "AAAAAAAAAAAAAAASdHJhbnNmZXJfb3duZXJzaGlwAAAAAAACAAAAAAAAAAluZXdfb3duZXIAAAAAAAATAAAAAAAAABFsaXZlX3VudGlsX2xlZGdlcgAAAAAAAAQAAAAA",
        "AAAAAAAAAAAAAAAQYWNjZXB0X293bmVyc2hpcAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAScmVub3VuY2Vfb3duZXJzaGlwAAAAAAAAAAAAAA==",
        "AAAAAQAAAAAAAAAAAAAABk1hcmtldAAAAAAAAwAAAAAAAAAFYXNzZXQAAAAAAAfQAAAABUFzc2V0AAAAAAAAAAAAAAZjb25maWcAAAAAB9AAAAAMTWFya2V0Q29uZmlnAAAAAAAAAARkYXRhAAAH0AAAAApNYXJrZXREYXRhAAA=",
        "AAAAAQAAAAAAAAAAAAAAB1JlcXVlc3QAAAAAAwAAAAAAAAAGYWN0aW9uAAAAAAfQAAAAC1JlcXVlc3RUeXBlAAAAAAAAAAAEZGF0YQAAA+gAAAALAAAAAAAAAAhwb3NpdGlvbgAAAAQ=",
        "AAAAAwAAAC9UaGUgdHlwZSBvZiByZXF1ZXN0IHRvIGJlIG1hZGUgYWdhaW5zdCB0aGUgcG9vbAAAAAAAAAAAC1JlcXVlc3RUeXBlAAAAAAoAAAAAAAAABUNsb3NlAAAAAAAAAAAAAAAAAAAERmlsbAAAAAEAAAAAAAAACFN0b3BMb3NzAAAAAgAAAAAAAAAKVGFrZVByb2ZpdAAAAAAAAwAAAAAAAAALTGlxdWlkYXRpb24AAAAABAAAAAAAAAAGQ2FuY2VsAAAAAAAFAAAAAAAAABFEZXBvc2l0Q29sbGF0ZXJhbAAAAAAAAAYAAAAAAAAAEldpdGhkcmF3Q29sbGF0ZXJhbAAAAAAABwAAAAAAAAANU2V0VGFrZVByb2ZpdAAAAAAAAAgAAAAAAAAAC1NldFN0b3BMb3NzAAAAAAk=",
        "AAAAAQAAAAAAAAAAAAAADFN1Ym1pdFJlc3VsdAAAAAIAAAAAAAAAB3Jlc3VsdHMAAAAD6gAAAAQAAAAAAAAACXRyYW5zZmVycwAAAAAAA+wAAAATAAAACw==",
        "AAAAAQAAAAAAAAAAAAAADVRyYWRpbmdDb25maWcAAAAAAAADAAAAAAAAABBjYWxsZXJfdGFrZV9yYXRlAAAACwAAAAAAAAANbWF4X3Bvc2l0aW9ucwAAAAAAAAQAAAAAAAAABm9yYWNsZQAAAAAAEw==",
        "AAAAAgAAAA9Qb3NpdGlvbiBzdGF0dXMAAAAAAAAAAA5Qb3NpdGlvblN0YXR1cwAAAAAAAwAAAAAAAAAAAAAAB1BlbmRpbmcAAAAAAAAAAAAAAAAET3BlbgAAAAAAAAAAAAAABkNsb3NlZAAA",
        "AAAAAQAAAAAAAAAAAAAADE1hcmtldENvbmZpZwAAAA0AAAAAAAAACGJhc2VfZmVlAAAACwAAAAAAAAAHZW5hYmxlZAAAAAABAAAAAAAAAAtpbml0X21hcmdpbgAAAAALAAAAAAAAABJtYWludGVuYW5jZV9tYXJnaW4AAAAAAAsAAAAAAAAADm1heF9jb2xsYXRlcmFsAAAAAAALAAAAAAAAAA9tYXhfaG91cmx5X3JhdGUAAAAACwAAAAAAAAAKbWF4X3BheW91dAAAAAAACwAAAAAAAAAObWluX2NvbGxhdGVyYWwAAAAAAAsAAAAAAAAAD21pbl9ob3VybHlfcmF0ZQAAAAALAAAAAAAAABNwcmljZV9pbXBhY3Rfc2NhbGFyAAAAAAsAAAAAAAAAEnRhcmdldF9ob3VybHlfcmF0ZQAAAAAACwAAAAAAAAASdGFyZ2V0X3V0aWxpemF0aW9uAAAAAAALAAAAAAAAAA90b3RhbF9hdmFpbGFibGUAAAAACw==",
        "AAAAAQAAAAAAAAAAAAAAEFF1ZXVlZE1hcmtldEluaXQAAAACAAAAAAAAAAZjb25maWcAAAAAB9AAAAAMTWFya2V0Q29uZmlnAAAAAAAAAAt1bmxvY2tfdGltZQAAAAAG",
        "AAAAAQAAAAAAAAAAAAAACk1hcmtldERhdGEAAAAAAAkAAAAAAAAAC2xhc3RfdXBkYXRlAAAAAAYAAAAAAAAAD2xvbmdfY29sbGF0ZXJhbAAAAAALAAAAAAAAAApsb25nX2NvdW50AAAAAAAEAAAAAAAAABNsb25nX2ludGVyZXN0X2luZGV4AAAAAAsAAAAAAAAAEmxvbmdfbm90aW9uYWxfc2l6ZQAAAAAACwAAAAAAAAAQc2hvcnRfY29sbGF0ZXJhbAAAAAsAAAAAAAAAC3Nob3J0X2NvdW50AAAAAAQAAAAAAAAAFHNob3J0X2ludGVyZXN0X2luZGV4AAAACwAAAAAAAAATc2hvcnRfbm90aW9uYWxfc2l6ZQAAAAAL",
        "AAAAAQAAAC9TdHJ1Y3R1cmUgdG8gc3RvcmUgaW5mb3JtYXRpb24gYWJvdXQgYSBwb3NpdGlvbgAAAAAAAAAACFBvc2l0aW9uAAAADQAAAAAAAAAFYXNzZXQAAAAAAAfQAAAABUFzc2V0AAAAAAAAAAAAAAtjbG9zZV9wcmljZQAAAAALAAAAAAAAAApjb2xsYXRlcmFsAAAAAAALAAAAAAAAAApjcmVhdGVkX2F0AAAAAAAGAAAAAAAAAAtlbnRyeV9wcmljZQAAAAALAAAAAAAAAAJpZAAAAAAABAAAAAAAAAAOaW50ZXJlc3RfaW5kZXgAAAAAAAsAAAAAAAAAB2lzX2xvbmcAAAAAAQAAAAAAAAANbm90aW9uYWxfc2l6ZQAAAAAAAAsAAAAAAAAABnN0YXR1cwAAAAAH0AAAAA5Qb3NpdGlvblN0YXR1cwAAAAAAAAAAAAlzdG9wX2xvc3MAAAAAAAALAAAAAAAAAAt0YWtlX3Byb2ZpdAAAAAALAAAAAAAAAAR1c2VyAAAAEw==",
        "AAAAAQAAAC9QcmljZSBkYXRhIGZvciBhbiBhc3NldCBhdCBhIHNwZWNpZmljIHRpbWVzdGFtcAAAAAAAAAAACVByaWNlRGF0YQAAAAAAAAIAAAAAAAAABXByaWNlAAAAAAAACwAAAAAAAAAJdGltZXN0YW1wAAAAAAAABg==",
        "AAAAAgAAAApBc3NldCB0eXBlAAAAAAAAAAAABUFzc2V0AAAAAAAAAgAAAAEAAAAAAAAAB1N0ZWxsYXIAAAAAAQAAABMAAAABAAAAAAAAAAVPdGhlcgAAAAAAAAEAAAAR",
        "AAAAAQAAADFTdG9yYWdlIGtleSBmb3IgZW51bWVyYXRpb24gb2YgYWNjb3VudHMgcGVyIHJvbGUuAAAAAAAAAAAAAA5Sb2xlQWNjb3VudEtleQAAAAAAAgAAAAAAAAAFaW5kZXgAAAAAAAAEAAAAAAAAAARyb2xlAAAAEQ==",
        "AAAAAgAAADxTdG9yYWdlIGtleXMgZm9yIHRoZSBkYXRhIGFzc29jaWF0ZWQgd2l0aCB0aGUgYWNjZXNzIGNvbnRyb2wAAAAAAAAAF0FjY2Vzc0NvbnRyb2xTdG9yYWdlS2V5AAAAAAYAAAABAAAAAAAAAAxSb2xlQWNjb3VudHMAAAABAAAH0AAAAA5Sb2xlQWNjb3VudEtleQAAAAAAAQAAAAAAAAAHSGFzUm9sZQAAAAACAAAAEwAAABEAAAABAAAAAAAAABFSb2xlQWNjb3VudHNDb3VudAAAAAAAAAEAAAARAAAAAQAAAAAAAAAJUm9sZUFkbWluAAAAAAAAAQAAABEAAAAAAAAAAAAAAAVBZG1pbgAAAAAAAAAAAAAAAAAADFBlbmRpbmdBZG1pbg==",
        "AAAABAAAAAAAAAAAAAAAEkFjY2Vzc0NvbnRyb2xFcnJvcgAAAAAACQAAAAAAAAAMVW5hdXRob3JpemVkAAAEugAAAAAAAAALQWRtaW5Ob3RTZXQAAAAEuwAAAAAAAAAQSW5kZXhPdXRPZkJvdW5kcwAABLwAAAAAAAAAEUFkbWluUm9sZU5vdEZvdW5kAAAAAAAEvQAAAAAAAAASUm9sZUNvdW50SXNOb3RaZXJvAAAAAAS+AAAAAAAAAAxSb2xlTm90Rm91bmQAAAS/AAAAAAAAAA9BZG1pbkFscmVhZHlTZXQAAAAEwAAAAAAAAAALUm9sZU5vdEhlbGQAAAAEwQAAAAAAAAALUm9sZUlzRW1wdHkAAAAEwg==",
        "AAAAAgAAACNTdG9yYWdlIGtleXMgZm9yIGBPd25hYmxlYCB1dGlsaXR5LgAAAAAAAAAAEU93bmFibGVTdG9yYWdlS2V5AAAAAAAAAgAAAAAAAAAAAAAABU93bmVyAAAAAAAAAAAAAAAAAAAMUGVuZGluZ093bmVy",
        "AAAABAAAAAAAAAAAAAAADE93bmFibGVFcnJvcgAAAAMAAAAAAAAAC093bmVyTm90U2V0AAAABMQAAAAAAAAAElRyYW5zZmVySW5Qcm9ncmVzcwAAAAAExQAAAAAAAAAPT3duZXJBbHJlYWR5U2V0AAAABMY=",
        "AAAABAAAAAAAAAAAAAAAEVJvbGVUcmFuc2ZlckVycm9yAAAAAAAAAwAAAAAAAAARTm9QZW5kaW5nVHJhbnNmZXIAAAAAAASwAAAAAAAAABZJbnZhbGlkTGl2ZVVudGlsTGVkZ2VyAAAAAASxAAAAAAAAABVJbnZhbGlkUGVuZGluZ0FjY291bnQAAAAAAASy" ]),
      options
    )
  }
  public readonly fromJSON = {
    initialize: this.txFromJSON<null>,
        set_config: this.txFromJSON<null>,
        queue_set_market: this.txFromJSON<null>,
        cancel_set_market: this.txFromJSON<null>,
        set_market: this.txFromJSON<null>,
        set_status: this.txFromJSON<null>,
        create_position: this.txFromJSON<u32>,
        submit: this.txFromJSON<SubmitResult>,
        upgrade: this.txFromJSON<null>,
        get_owner: this.txFromJSON<Option<string>>,
        transfer_ownership: this.txFromJSON<null>,
        accept_ownership: this.txFromJSON<null>,
        renounce_ownership: this.txFromJSON<null>
  }
}