// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

interface IClaimVerifier {
    enum ClaimState {
        None,
        Pending,
        Settled,
        Slashed
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

    event TracePublished(
        address indexed publisher,
        bytes32 indexed payloadHash,
        uint32 payloadBytes,
        uint8 codecId
    );

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

    function challengePeriod() external view returns (uint64);

    function minBond() external view returns (uint256);
}
