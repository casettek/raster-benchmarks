use alloy::sol;

sol! {
    #[sol(rpc)]
    contract ClaimVerifier {
        constructor(uint64 _challengePeriod, uint256 _minBond, uint64 _blobRetentionWindow);
    }
}

#[allow(clippy::too_many_arguments)]
mod claim_verifier_interface {
    use alloy::sol;

    sol! {
        #[sol(rpc)]
        interface IClaimVerifier {
            enum ClaimState {
                None,
                Pending,
                Settled,
                Slashed,
            }

            struct BlobRegistration {
                uint64 blockNumber;
                uint64 timestamp;
            }

            struct Claim {
                address claimer;
                bytes32 prevOutputRoot;
                bytes32 nextOutputRoot;
                uint64 startBlock;
                uint64 endBlock;
                bytes32 batchHash;
                bytes32 inputBlobVersionedHash;
                bytes32 traceBlobVersionedHash;
                uint256 bondAmount;
                uint64 createdAt;
                uint64 challengeDeadline;
                ClaimState state;
            }

            event ClaimSubmitted(
                uint256 indexed claimId,
                address indexed claimer,
                bytes32 prevOutputRoot,
                bytes32 nextOutputRoot,
                uint64 startBlock,
                uint64 endBlock,
                bytes32 batchHash,
                bytes32 inputBlobVersionedHash,
                bytes32 traceBlobVersionedHash,
                uint256 bondAmount,
                uint64 challengeDeadline
            );

            event ClaimChallenged(
                uint256 indexed claimId,
                address indexed challenger,
                bytes32 observedNextOutputRoot
            );

            event ClaimSettled(uint256 indexed claimId);

            event ClaimSlashed(uint256 indexed claimId);

            event BlobRegistered(
                bytes32 indexed blobVersionedHash,
                uint64 blockNumber,
                uint64 timestamp
            );

            event BlobAlreadyRegistered(
                bytes32 indexed blobVersionedHash,
                uint64 originalBlockNumber,
                uint64 originalTimestamp
            );

            function registerManifestBlobs() external;

            function submitClaim(
                bytes32 prevOutputRoot,
                bytes32 nextOutputRoot,
                uint64 startBlock,
                uint64 endBlock,
                bytes32 batchHash,
                bytes32 inputBlobVersionedHash,
                bytes32 traceBlobVersionedHash
            ) external payable returns (uint256 claimId);

            function challengeClaim(
                uint256 claimId,
                bytes32 observedNextOutputRoot
            ) external;

            function settleClaim(uint256 claimId) external;

            function getClaim(uint256 claimId) external view returns (Claim memory);

            function getBlobRegistration(bytes32 blobVersionedHash)
                external
                view
                returns (BlobRegistration memory);

            function challengePeriod() external view returns (uint64);

            function minBond() external view returns (uint256);

            function blobRetentionWindow() external view returns (uint64);
        }
    }
}

pub use claim_verifier_interface::IClaimVerifier;
