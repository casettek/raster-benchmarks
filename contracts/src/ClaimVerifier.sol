// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {IClaimVerifier} from "./interfaces/IClaimVerifier.sol";

contract ClaimVerifier is IClaimVerifier {
    uint256 private _nextClaimId = 1;
    mapping(uint256 => Claim) private _claims;

    function publishTrace(
        bytes calldata payload,
        uint8 codecId
    ) external returns (bytes32 payloadHash, uint32 payloadBytes) {
        payloadHash = keccak256(payload);
        payloadBytes = uint32(payload.length);
        emit TracePublished(msg.sender, payloadHash, payloadBytes, codecId);
    }

    function submitClaim(
        bytes32 workloadId,
        bytes32 artifactRoot,
        bytes32 resultRoot,
        bytes32 traceTxHash,
        uint32 tracePayloadBytes,
        uint8 traceCodecId
    ) external returns (uint256 claimId) {
        claimId = _nextClaimId++;
        _claims[claimId] = Claim({
            claimer: msg.sender,
            workloadId: workloadId,
            artifactRoot: artifactRoot,
            resultRoot: resultRoot,
            traceTxHash: traceTxHash,
            tracePayloadBytes: tracePayloadBytes,
            traceCodecId: traceCodecId,
            createdAt: uint64(block.timestamp),
            state: ClaimState.Pending
        });

        emit ClaimSubmitted(
            claimId,
            msg.sender,
            workloadId,
            artifactRoot,
            resultRoot,
            traceTxHash,
            tracePayloadBytes,
            traceCodecId
        );
    }

    function challengeClaim(
        uint256 claimId,
        bytes32 observedArtifactRoot,
        bytes32 observedResultRoot
    ) external {
        Claim storage claim = _claims[claimId];
        require(claim.state == ClaimState.Pending, "claim not pending");

        bool mismatch = claim.artifactRoot != observedArtifactRoot
            || claim.resultRoot != observedResultRoot;
        require(mismatch, "no divergence detected");

        claim.state = ClaimState.Slashed;
        emit ClaimChallenged(
            claimId,
            msg.sender,
            observedArtifactRoot,
            observedResultRoot
        );
        emit ClaimSlashed(claimId);
    }

    function settleClaim(uint256 claimId) external {
        Claim storage claim = _claims[claimId];
        require(claim.state == ClaimState.Pending, "claim not pending");

        claim.state = ClaimState.Settled;
        emit ClaimSettled(claimId);
    }

    function getClaim(uint256 claimId) external view returns (Claim memory) {
        return _claims[claimId];
    }
}
