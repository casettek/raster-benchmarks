// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {IClaimVerifier} from "./interfaces/IClaimVerifier.sol";

contract ClaimVerifier is IClaimVerifier {
    uint256 private _nextClaimId = 1;
    mapping(uint256 => Claim) private _claims;

    /// @notice Configurable challenge period in seconds (default 120s for local demo).
    uint64 public override challengePeriod = 120;

    /// @notice Minimum bond required to submit a claim (default 0.01 ether for local demo).
    uint256 public override minBond = 0.01 ether;

    /// @notice Constructor allows overriding defaults for local Anvil testing.
    constructor(uint64 _challengePeriod, uint256 _minBond) {
        challengePeriod = _challengePeriod;
        minBond = _minBond;
    }

    function publishTrace(
        bytes calldata payload,
        uint8 codecId
    ) external returns (bytes32 payloadHash, uint32 payloadBytes) {
        payloadHash = keccak256(payload);
        payloadBytes = uint32(payload.length);
        emit TracePublished(msg.sender, payloadHash, payloadBytes, codecId);
    }

    function submitClaim(
        bytes32 prevOutputRoot,
        bytes32 nextOutputRoot,
        uint64 startBlock,
        uint64 endBlock,
        bytes32 batchHash,
        bytes32 traceTxHash,
        uint32 tracePayloadBytes,
        uint8 traceCodecId
    ) external payable returns (uint256 claimId) {
        require(msg.value >= minBond, "insufficient bond");
        require(startBlock <= endBlock, "invalid block range");
        require(traceTxHash != bytes32(0), "missing trace tx hash");
        require(tracePayloadBytes > 0, "missing trace payload");
        require(traceCodecId != 0, "missing trace codec");

        uint64 deadline = uint64(block.timestamp) + challengePeriod;

        // Capture the blob versioned hash from the current tx context.
        // On local Anvil without real blobs this will be bytes32(0).
        bytes32 blobHash = _currentBlobVersionedHash();

        claimId = _nextClaimId++;
        _claims[claimId] = Claim({
            claimer: msg.sender,
            prevOutputRoot: prevOutputRoot,
            nextOutputRoot: nextOutputRoot,
            startBlock: startBlock,
            endBlock: endBlock,
            batchHash: batchHash,
            inputBlobVersionedHash: blobHash,
            traceTxHash: traceTxHash,
            tracePayloadBytes: tracePayloadBytes,
            traceCodecId: traceCodecId,
            bondAmount: msg.value,
            createdAt: uint64(block.timestamp),
            challengeDeadline: deadline,
            state: ClaimState.Pending
        });

        emit ClaimSubmitted(
            claimId,
            msg.sender,
            prevOutputRoot,
            nextOutputRoot,
            startBlock,
            endBlock,
            batchHash,
            blobHash,
            msg.value,
            deadline
        );
    }

    function challengeClaim(
        uint256 claimId,
        bytes32 observedNextOutputRoot
    ) external {
        Claim storage claim = _claims[claimId];
        require(claim.state == ClaimState.Pending, "claim not pending");
        require(
            uint64(block.timestamp) <= claim.challengeDeadline,
            "challenge period expired"
        );

        bool mismatch = claim.nextOutputRoot != observedNextOutputRoot;
        require(mismatch, "no divergence detected");

        claim.state = ClaimState.Slashed;
        emit ClaimChallenged(claimId, msg.sender, observedNextOutputRoot);
        emit ClaimSlashed(claimId);

        // Release bond to challenger (simplified v1: no challenger stake required)
        (bool sent, ) = payable(msg.sender).call{value: claim.bondAmount}("");
        require(sent, "bond transfer failed");
    }

    function settleClaim(uint256 claimId) external {
        Claim storage claim = _claims[claimId];
        require(claim.state == ClaimState.Pending, "claim not pending");
        require(
            uint64(block.timestamp) >= claim.challengeDeadline,
            "challenge period not expired"
        );

        claim.state = ClaimState.Settled;
        emit ClaimSettled(claimId);

        // Release bond back to claimer
        (bool sent, ) = payable(claim.claimer).call{value: claim.bondAmount}("");
        require(sent, "bond release failed");
    }

    function getClaim(uint256 claimId) external view returns (Claim memory) {
        return _claims[claimId];
    }

    /// @dev Attempt to read the first blob versioned hash from EIP-4844 context.
    /// Returns bytes32(0) on chains/environments without blob support (e.g., Anvil).
    function _currentBlobVersionedHash() private view returns (bytes32 hash) {
        // BLOBHASH opcode (0x49) with index 0
        // On environments without blob support, this returns 0.
        assembly {
            hash := blobhash(0)
        }
    }
}
