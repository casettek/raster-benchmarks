// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {IClaimVerifier} from "./interfaces/IClaimVerifier.sol";

contract ClaimVerifier is IClaimVerifier {
    uint256 private _nextClaimId = 1;
    mapping(uint256 => Claim) private _claims;
    mapping(bytes32 => BlobRegistration) private _blobRegistrations;

    uint256 private constant _MAX_BLOB_COUNT = 6;

    /// @notice Configurable challenge period in seconds (default 120s for local demo).
    uint64 public override challengePeriod = 120;

    /// @notice Minimum bond required to submit a claim (default 0.01 ether for local demo).
    uint256 public override minBond = 0.01 ether;

    /// @notice Blob availability window in seconds (default 18 days for local demo).
    uint64 public override blobRetentionWindow = 18 days;

    /// @notice Constructor allows overriding defaults for local Anvil testing.
    constructor(
        uint64 _challengePeriod,
        uint256 _minBond,
        uint64 _blobRetentionWindow
    ) {
        require(
            _blobRetentionWindow >= _challengePeriod,
            "blob retention shorter than challenge period"
        );
        challengePeriod = _challengePeriod;
        minBond = _minBond;
        blobRetentionWindow = _blobRetentionWindow;
    }

    function registerManifestBlobs() external {
        bool sawBlob;
        for (uint256 i = 0; i < _MAX_BLOB_COUNT; i++) {
            bytes32 versionedHash = blobhash(i);
            if (versionedHash == bytes32(0)) {
                break;
            }
            sawBlob = true;

            BlobRegistration storage existing = _blobRegistrations[versionedHash];
            if (existing.timestamp != 0) {
                emit BlobAlreadyRegistered(
                    versionedHash,
                    existing.blockNumber,
                    existing.timestamp
                );
                continue;
            }

            BlobRegistration memory registration = BlobRegistration({
                blockNumber: uint64(block.number),
                timestamp: uint64(block.timestamp)
            });
            _blobRegistrations[versionedHash] = registration;
            emit BlobRegistered(
                versionedHash,
                registration.blockNumber,
                registration.timestamp
            );
        }

        require(sawBlob, "missing blob hash");
    }

    function submitClaim(
        bytes32 prevOutputRoot,
        bytes32 nextOutputRoot,
        uint64 startBlock,
        uint64 endBlock,
        bytes32 batchHash,
        bytes32 inputBlobVersionedHash,
        bytes32 traceBlobVersionedHash
    ) external payable returns (uint256 claimId) {
        require(msg.value >= minBond, "insufficient bond");
        require(startBlock <= endBlock, "invalid block range");

        _requireRegisteredFresh(
            traceBlobVersionedHash,
            "missing trace blob versioned hash"
        );
        if (inputBlobVersionedHash != bytes32(0)) {
            _requireRegisteredFresh(
                inputBlobVersionedHash,
                "missing input blob versioned hash"
            );
        }

        uint64 deadline = uint64(block.timestamp) + challengePeriod;

        claimId = _nextClaimId++;
        _claims[claimId] = Claim({
            claimer: msg.sender,
            prevOutputRoot: prevOutputRoot,
            nextOutputRoot: nextOutputRoot,
            startBlock: startBlock,
            endBlock: endBlock,
            batchHash: batchHash,
            inputBlobVersionedHash: inputBlobVersionedHash,
            traceBlobVersionedHash: traceBlobVersionedHash,
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
            inputBlobVersionedHash,
            traceBlobVersionedHash,
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

    function getBlobRegistration(
        bytes32 blobVersionedHash
    ) external view returns (BlobRegistration memory) {
        return _blobRegistrations[blobVersionedHash];
    }

    function _requireRegisteredFresh(
        bytes32 blobVersionedHash,
        string memory zeroHashMessage
    ) internal view {
        require(blobVersionedHash != bytes32(0), zeroHashMessage);

        BlobRegistration memory registration = _blobRegistrations[
            blobVersionedHash
        ];
        require(registration.timestamp != 0, "unregistered blob versioned hash");

        uint256 requiredUntil = uint256(block.timestamp) + challengePeriod;
        uint256 availableUntil =
            uint256(registration.timestamp) + blobRetentionWindow;
        require(
            availableUntil >= requiredUntil,
            "blob versioned hash too old"
        );
    }
}
