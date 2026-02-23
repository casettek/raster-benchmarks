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
        bytes32 workloadId;
        bytes32 artifactRoot;
        bytes32 resultRoot;
        uint64 createdAt;
        ClaimState state;
    }

    function submitClaim(
        bytes32 workloadId,
        bytes32 artifactRoot,
        bytes32 resultRoot
    ) external returns (uint256 claimId);

    function challengeClaim(
        uint256 claimId,
        bytes32 observedArtifactRoot,
        bytes32 observedResultRoot
    ) external;

    function settleClaim(uint256 claimId) external;

    function getClaim(uint256 claimId) external view returns (Claim memory);
}
